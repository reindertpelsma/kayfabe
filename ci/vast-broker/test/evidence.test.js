import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import { setup, callbackToken, env } from './helpers.js';
import { createGateway } from '../src/index.js';
import { routeKind } from '../src/routes.js';
import {
  EVIDENCE_MAX_BYTES,
  evidenceEnabled,
  evidenceKeys,
  evidenceTimeouts,
  parseEvidenceMeta,
} from '../src/evidence.js';
// FALSIFIER: POST /evidence/<run> stores a second bundle for a rental, stores anything for a caller
// without that rental's live run token, reads a body whose declared length is missing or above 8 MiB,
// stores a body whose gzip magic or declared SHA-256 is wrong or whose metadata is not the fixed
// shape, answers when the bucket binding is absent with anything but 404, puts a token or the
// bundle bytes into a response, a log line or the manifest, changes a verdict, a deadline or the
// budget, or makes the bucket readable from any other route.
const run = '123-1';
class Bucket {
  objects = new Map();
  puts = 0;
  fail = false;
  async head(key) {
    return this.objects.has(key) ? { key } : null;
  }
  async put(key, value, options = {}) {
    this.puts++;
    if (this.fail) throw new Error('R2 down');
    const bytes =
      typeof value === 'string' ? new TextEncoder().encode(value) : new Uint8Array(value);
    this.objects.set(key, { bytes, options });
    return { key };
  }
}
const sha = (bytes) => createHash('sha256').update(bytes).digest('hex');
const META = {
  v: 1,
  sections: [{ name: 'log', files: 1, raw_bytes: 100, dropped: 0 }],
  dropped: 0,
  raw_bytes: 100,
  scrub: { uuid: 1, serial: 0, mac: 0, ipv4: 2, ipv6: 0 },
  scan: 'clean',
};
const bundle = (text = 'evidence text\n') => gzipSync(Buffer.from(text));
// The whole path: the stateless gateway reads and writes the bucket; the ledger service (the
// Durable Object's logic) only reserves and settles. `h.service` is the service transport.
async function harness(extra = {}, { bucket = new Bucket() } = {}) {
  const h = await setup(extra.noBucket ? {} : { EVIDENCE_BUCKET: bucket, ...extra });
  await h.call();
  const live = await h.storage.get('record:' + run);
  live.bootDeadline = h.time.value + 3600000;
  live.hardDeadline = h.time.value + 7200000;
  await h.storage.put('record:' + run, live);
  const token = callbackToken(live);
  const internal = [],
    sizes = [],
    forwarded = [],
    hold = {};
  const ledger = {
    idFromName: () => 1,
    get: () => ({
      fetch: async (request) => {
        const url = new URL(request.url);
        if (url.origin === 'https://internal.invalid') {
          internal.push(url.pathname);
          sizes.push((await request.clone().text()).length);
          if (hold.reserve && url.pathname === '/evidence/reserve') await hold.reserve;
          const call = url.pathname === '/evidence/reserve' ? 'evidenceReserve' : 'evidenceSettle';
          try {
            return Response.json(await h.service[call](request));
          } catch (e) {
            return Response.json({ state: e.state ?? 'UNAVAILABLE' }, { status: e.status ?? 503 });
          }
        }
        forwarded.push(url.pathname);
        return h.service.fetch(request);
      },
    }),
  };
  const gateway = createGateway({ now: () => h.time.value });
  const gatewayEnv = { ...h.bindings, LEDGER: ledger };
  const prefix = '/' + h.bindings.BROKER_PATH_PREFIX;
  const raw = (path, init) =>
    gateway.fetch(new Request(h.bindings.WORKER_URL + prefix + path, init), gatewayEnv);
  return {
    h,
    bucket,
    token,
    internal,
    sizes,
    forwarded,
    hold,
    raw,
    gateway,
    async post(
      body = bundle(),
      {
        t = token,
        path = '/evidence/' + run,
        meta = META,
        digest,
        length = body?.length,
        headers = {},
        method = 'POST',
      } = {},
    ) {
      const h2 = {
        ...(t === null ? {} : { Authorization: 'Bearer ' + t }),
        'Content-Type': 'application/gzip',
        'X-KF-Evidence-SHA256': digest ?? (body ? sha(body) : sha(Buffer.alloc(0))),
        ...(meta === null ? {} : { 'X-KF-Evidence-Meta': JSON.stringify(meta) }),
        ...(length === undefined ? {} : { 'Content-Length': String(length) }),
        ...headers,
      };
      return raw(path, { method, headers: h2, body });
    },
    // The ledger step alone, as the gateway calls it.
    reserve(body = { key: run, length: 100 }, t = token) {
      return h.service.evidenceReserve(
        new Request('https://internal.invalid/evidence/reserve', {
          method: 'POST',
          headers: { Authorization: 'Bearer ' + t },
          body: JSON.stringify(body),
        }),
      );
    },
    settle(body) {
      return h.service.evidenceSettle(
        new Request('https://internal.invalid/evidence/settle', {
          method: 'POST',
          body: JSON.stringify(body),
        }),
      );
    },
  };
}
const everything = async (r) => JSON.stringify([...r.headers]) + (await r.clone().text());

test('evidence: a live run token stores one bundle and its manifest under <profile>/<run>/<attempt>', async () => {
  const x = await harness();
  const body = bundle('line one\nline two\n');
  const r = await x.post(body);
  assert.equal(r.status, 200);
  const text = await r.text();
  const answer = JSON.parse(text);
  assert.equal(answer.state, 'STORED');
  assert.equal(answer.size, body.length);
  assert.equal(answer.sha256, sha(body));
  assert.deepEqual([...x.bucket.objects.keys()].sort(), ['check/123/1.json', 'check/123/1.tgz']);
  const stored = x.bucket.objects.get('check/123/1.tgz');
  assert.deepEqual(Buffer.from(stored.bytes), body);
  assert.equal(stored.options.httpMetadata.contentType, 'application/gzip');
  const manifest = JSON.parse(Buffer.from(x.bucket.objects.get('check/123/1.json').bytes));
  assert.equal(manifest.sha256, sha(body));
  assert.equal(manifest.size, body.length);
  assert.equal(manifest.run, '123');
  assert.equal(manifest.attempt, '1');
  assert.equal(manifest.profile, 'check');
  assert.equal(manifest.scan, 'clean');
  assert.deepEqual(manifest.scrub, META.scrub);
  assert.deepEqual(manifest.box_claim.sections, META.sections);
  assert.match(manifest.uploaded_at, /^\d{4}-\d\d-\d\dT/);
  assert.equal(r.headers.get('Cache-Control'), 'no-store');
  const seen =
    JSON.stringify([...r.headers]) +
    text +
    JSON.stringify(manifest) +
    JSON.stringify(stored.options);
  assert.ok(!seen.includes(x.token));
  assert.ok(!seen.includes(env.RUN_TOKEN_SECRET));
});
test('evidence: the key carries the rental profile', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:' + run);
  r.profile = 'sweep';
  await x.h.storage.put('record:' + run, r);
  assert.equal((await x.post()).status, 200);
  assert.ok(x.bucket.objects.has('sweep/123/1.tgz'));
  assert.ok(x.bucket.objects.has('sweep/123/1.json'));
});
test('evidence: keys name profile, run and attempt, and refuse a malformed one', () => {
  assert.deepEqual(evidenceKeys({ key: '7-3', budgetClass: 'release' }).object, 'release/7/3.tgz');
  assert.throws(() => evidenceKeys({ key: '../x', profile: 'check' }));
  assert.throws(() => evidenceKeys({ key: '7-3', profile: '../x' }));
});
test('evidence: a second upload for the same rental is 409 and changes nothing', async () => {
  const x = await harness();
  assert.equal((await x.post(bundle('first'))).status, 200);
  const before = Buffer.from(x.bucket.objects.get('check/123/1.tgz').bytes);
  const again = await x.post(bundle('second, longer text'));
  assert.equal(again.status, 409);
  assert.equal((await again.json()).state, 'EVIDENCE_EXISTS');
  assert.deepEqual(Buffer.from(x.bucket.objects.get('check/123/1.tgz').bytes), before);
});
test('evidence: stored after the result, during the grace window', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:' + run);
  r.resultAt = x.h.time.value;
  r.verdict = 'FAIL';
  r.phase = 'GRACE';
  r.resultDeadline = x.h.time.value + 600000;
  await x.h.storage.put('record:' + run, r);
  assert.equal((await x.post()).status, 200);
  const after = await x.h.storage.get('record:' + run);
  assert.equal(after.verdict, 'FAIL');
  assert.equal(after.resultAt, r.resultAt);
});
for (const mode of [
  'missing',
  'bad',
  'wrong-run',
  'other-run-path',
  'closed',
  'ended',
  'expired',
  'result-deadline',
])
  test('evidence: the run token is required and the rental live: ' + mode, async () => {
    const x = await harness();
    let t = x.token,
      path = '/evidence/' + run;
    if (mode === 'missing') t = null;
    if (mode === 'bad') t = 'bad';
    if (mode === 'wrong-run')
      t = callbackToken(await x.h.storage.get('record:' + run), undefined, { run: '124' });
    if (mode === 'other-run-path') path = '/evidence/124-1';
    if (['closed', 'ended', 'expired', 'result-deadline'].includes(mode)) {
      const r = await x.h.storage.get('record:' + run);
      if (mode === 'closed') r.closed = 'STOPPED';
      else if (mode === 'ended') r.ended = x.h.time.value;
      else if (mode === 'expired') x.h.time.value = r.hardDeadline;
      else {
        r.resultAt = x.h.time.value - 7200000;
        r.resultDeadline = x.h.time.value - 1000;
      }
      await x.h.storage.put('record:' + run, r);
    }
    const res = await x.post(bundle(), { t, path });
    assert.ok([401, 403].includes(res.status), String(res.status));
    assert.equal(x.bucket.puts, 0);
  });
test('evidence: without the bucket binding the route is 404 and the box can go on', async () => {
  const x = await harness({ noBucket: true });
  const before = JSON.stringify(await x.h.storage.get('record:' + run));
  const r = await x.post();
  assert.equal(r.status, 404);
  assert.equal((await r.json()).state, 'EVIDENCE_DISABLED');
  assert.equal(JSON.stringify(await x.h.storage.get('record:' + run)), before);
});
test('evidence: a bad token is still 401 when the bucket is absent, and the binding is checked', async () => {
  const x = await harness({ noBucket: true });
  assert.equal((await x.post(bundle(), { t: 'bad' })).status, 401);
  assert.equal((await x.post(bundle(), { t: null })).status, 401);
  for (const value of [undefined, null, {}, 'bucket', 7])
    assert.equal(evidenceEnabled({ EVIDENCE_BUCKET: value }), false);
  assert.equal(evidenceEnabled({ EVIDENCE_BUCKET: new Bucket() }), true);
});
test('evidence: a declared length is required and capped before the body is read', async () => {
  const x = await harness();
  const big = new Uint8Array(EVIDENCE_MAX_BYTES + 1);
  big[0] = 0x1f;
  big[1] = 0x8b;
  assert.equal((await x.post(big)).status, 413);
  assert.equal((await x.post(bundle(), { length: EVIDENCE_MAX_BYTES + 1 })).status, 413);
  // no declared length: Request(body: bytes) would add one, so the header is removed explicitly
  const noLength = await x.raw('/evidence/' + run, {
    method: 'POST',
    headers: {
      Authorization: 'Bearer ' + x.token,
      'X-KF-Evidence-SHA256': sha(bundle()),
      'X-KF-Evidence-Meta': JSON.stringify(META),
      'Content-Length': 'abc',
    },
    body: bundle(),
  });
  assert.equal(noLength.status, 411);
  assert.equal((await x.post(bundle(), { length: 0 })).status, 411);
  assert.equal(x.bucket.puts, 0);
  // exactly the cap is accepted
  const edge = new Uint8Array(EVIDENCE_MAX_BYTES);
  edge.set(bundle());
  assert.equal((await x.post(edge)).status, 200);
});
for (const [name, body, opts] of [
  ['not-gzip', () => Buffer.from('plain text, not gzip at all'), {}],
  ['sha-mismatch', () => bundle(), { digest: 'a'.repeat(64) }],
  ['sha-malformed', () => bundle(), { digest: 'zz' }],
  ['sha-absent', () => bundle(), { digest: '' }],
  ['length-mismatch', () => bundle('x'.repeat(100)), { length: 25 }],
  ['tiny', () => Buffer.from([0x1f, 0x8b]), {}],
  ['zeros', () => Buffer.alloc(64), {}],
  [
    'looks-like-gzip-trailer-first',
    () => Buffer.concat([Buffer.from([0x8b, 0x1f]), Buffer.alloc(40)]),
    {},
  ],
])
  test('evidence: a body that is not the declared gzip bundle is refused: ' + name, async () => {
    const x = await harness();
    const r = await x.post(body(), opts);
    assert.equal(r.status, 400, name);
    assert.equal(x.bucket.puts, 0);
  });
const BAD_META = {
  absent: null,
  'not-json': 'not json',
  extra: { ...META, note: 'free text' },
  'scan-hit': { ...META, scan: 'hit' },
  'scan-missing': { ...META, scan: undefined },
  'version-2': { ...META, v: 2 },
  'negative-count': { ...META, dropped: -1 },
  'section-name': { ...META, sections: [{ ...META.sections[0], name: '../x' }] },
  'section-extra': { ...META, sections: [{ ...META.sections[0], path: '/etc' }] },
  'too-many-sections': { ...META, sections: Array.from({ length: 17 }, () => META.sections[0]) },
  'scrub-extra': { ...META, scrub: { ...META.scrub, token: 1 } },
  'scrub-missing': { ...META, scrub: { uuid: 0 } },
  'scrub-string': { ...META, scrub: { ...META.scrub, mac: '1' } },
  'sections-not-array': { ...META, sections: 'log' },
  'section-name-long': { ...META, sections: [{ ...META.sections[0], name: 'a'.repeat(25) }] },
  'section-name-upper': { ...META, sections: [{ ...META.sections[0], name: 'Log' }] },
  'raw-bytes-huge': { ...META, raw_bytes: 2 ** 40 },
};
for (const [name, meta] of Object.entries(BAD_META))
  test('evidence: the metadata header has the fixed shape: ' + name, async () => {
    const x = await harness();
    const body = bundle();
    const r =
      typeof meta === 'string'
        ? await x.post(body, { meta: null, headers: { 'X-KF-Evidence-Meta': meta } })
        : await x.post(body, { meta });
    assert.equal(r.status, 400, name);
    assert.equal(x.bucket.puts, 0);
  });
test('evidence: a metadata header above 4 KiB is refused, a good one parses', async () => {
  const x = await harness();
  const huge = JSON.stringify({ ...META, sections: [], pad: 'x'.repeat(5000) });
  assert.equal(
    (await x.post(bundle(), { meta: null, headers: { 'X-KF-Evidence-Meta': huge } })).status,
    400,
  );
  assert.equal(x.bucket.puts, 0);
  assert.throws(() => parseEvidenceMeta('{"v":1'));
  assert.equal(parseEvidenceMeta(JSON.stringify(META)).scan, 'clean');
});
test('evidence: a failing bucket is 503, writes nothing the next try cannot repeat', async () => {
  const x = await harness();
  x.bucket.fail = true;
  const before = JSON.stringify(await x.h.storage.get('record:' + run));
  const r = await x.post();
  assert.equal(r.status, 503);
  assert.equal(x.bucket.objects.size, 0);
  assert.equal(JSON.stringify(await x.h.storage.get('record:' + run)).includes('evidence'), false);
  assert.equal(
    JSON.parse(JSON.stringify(await x.h.storage.get('record:' + run))).verdict,
    JSON.parse(before).verdict,
  );
  x.bucket.fail = false;
  assert.equal((await x.post()).status, 200);
});
test('evidence: a failing manifest write still reports the stored bundle', async () => {
  const x = await harness();
  const put = x.bucket.put.bind(x.bucket);
  x.bucket.put = async (key, ...rest) => {
    if (key.endsWith('.json')) throw new Error('manifest down');
    return put(key, ...rest);
  };
  assert.equal((await x.post()).status, 200);
  assert.deepEqual([...x.bucket.objects.keys()], ['check/123/1.tgz']);
  assert.equal((await x.post()).status, 409);
});
test('evidence: only POST, no query string, and no route lists or serves the bucket', async () => {
  const x = await harness();
  for (const method of ['GET', 'PUT', 'DELETE']) {
    const r = await x
      .post(method === 'GET' ? undefined : bundle(), {
        method,
        length: method === 'GET' ? undefined : bundle().length,
      })
      .catch(() => null);
    assert.ok(r === null || r.status === 404, method);
  }
  assert.equal((await x.post(bundle(), { path: '/evidence/' + run + '?x=1' })).status, 404);
  assert.equal(x.bucket.puts, 0);
  for (const path of [
    '/evidence',
    '/evidence/',
    '/evidence/123',
    '/evidence/123-1/extra',
    '/evidence/list',
  ]) {
    const r = await x.post(bundle(), { path });
    assert.ok([401, 404].includes(r.status), path + ' ' + r.status);
  }
  assert.equal(routeKind('/evidence/123-1'), 'box');
  assert.equal(routeKind('/evidence/123-1/extra'), null);
  assert.equal(routeKind('/evidence'), null);
  await x.post();
  // the status route of the runner never mentions the bundle
  const status = await x.h.service.fetch(
    new Request(x.h.bindings.WORKER_URL + '/status/' + run, {
      headers: { Authorization: 'Bearer ' + (await (await import('./helpers.js')).jwt({})) },
    }),
  );
  assert.ok(!(await status.text()).includes('evidence'));
});
test('evidence: the per-run request limit applies and no provider call is made', async () => {
  const x = await harness();
  const calls = x.h.cloud.calls.length;
  const answers = [];
  for (let i = 0; i < 21; i++) answers.push((await x.post()).status);
  assert.deepEqual(answers.slice(0, 2), [200, 409]);
  assert.equal(answers[20], 429);
  assert.equal(x.h.cloud.calls.length, calls, 'no provider or GitHub call');
});
test('evidence: the other ledger state, the deadlines and the verdict are untouched', async () => {
  const x = await harness();
  const others = (m) => JSON.stringify([...m].filter(([k]) => k !== 'record:' + run));
  const before = await x.h.storage.get('record:' + run);
  const beforeOthers = others(x.h.storage.data);
  assert.equal((await x.post()).status, 200);
  const after = await x.h.storage.get('record:' + run);
  const volatile = ['rateWindow', 'rateCount', 'phase', 'evidence'];
  for (const k of new Set([...Object.keys(before), ...Object.keys(after)]))
    if (!volatile.includes(k)) assert.deepEqual(after[k], before[k], k);
  assert.equal(others(x.h.storage.data), beforeOthers);
  assert.equal(after.evidence.state, 'STORED');
  assert.match(after.evidence.sha256, /^[0-9a-f]{64}$/);
  assert.deepEqual(Object.keys(after.evidence).sort(), ['at', 'sha256', 'state']);
});
test('evidence: the cap is exactly 8 MiB', () => {
  assert.equal(EVIDENCE_MAX_BYTES, 8388608);
});
test('evidence: 9 MiB declared is refused before anything is read', async () => {
  const x = await harness();
  assert.equal((await x.post(bundle(), { length: 9 * 1024 * 1024 })).status, 413);
  assert.equal((await x.post(bundle(), { length: 8388609 })).status, 413);
  assert.equal(x.bucket.puts, 0);
  assert.deepEqual(x.internal, [], 'refused by the gateway, the ledger is not asked');
});
test('evidence: disabled mode answers 404 after authentication and before the body is read', async () => {
  const x = await harness({ noBucket: true });
  let pulled = 0;
  const stream = new ReadableStream(
    {
      pull(controller) {
        pulled++;
        controller.enqueue(new Uint8Array(1024));
      },
    },
    { highWaterMark: 0 },
  );
  const r = await x.raw('/evidence/' + run, {
    method: 'POST',
    duplex: 'half',
    headers: {
      Authorization: 'Bearer ' + x.token,
      'Content-Length': '1000000',
      'X-KF-Evidence-SHA256': 'zz',
      'X-KF-Evidence-Meta': 'not json',
    },
    body: stream,
  });
  assert.equal(r.status, 404);
  assert.equal((await r.json()).state, 'EVIDENCE_DISABLED');
  assert.equal(pulled, 0);
  assert.deepEqual(x.internal, ['/evidence/reserve'], 'no settle: nothing was reserved');
});
test('evidence: a binding without put() is the same as no binding', async () => {
  const x = await harness({ EVIDENCE_BUCKET: {}, noBucket: false }, { bucket: {} });
  const r = await x.post();
  assert.equal(r.status, 404);
  assert.equal((await r.json()).state, 'EVIDENCE_DISABLED');
});
test('evidence: a closed rental is refused after the result too', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:' + run);
  r.resultAt = x.h.time.value;
  r.phase = 'GRACE';
  r.closed = 'DEADLINE';
  await x.h.storage.put('record:' + run, r);
  assert.equal((await x.post()).status, 401);
  assert.equal(x.bucket.puts, 0);
});
test('evidence: the ledger counts requests per rental: 429 after 20 in a window, then it reopens', async () => {
  const x = await harness();
  const status = async () => {
    try {
      await x.reserve();
      return 200;
    } catch (e) {
      return e.status;
    }
  };
  const answers = [];
  for (let i = 0; i < 21; i++) answers.push(await status());
  assert.deepEqual(answers.slice(0, 2), [200, 409]);
  assert.equal(answers[20], 429);
  x.h.time.value += 10000;
  assert.equal(await status(), 409);
});
test('evidence: the manifest holds exactly these fields and the object these options', async () => {
  const x = await harness();
  assert.equal((await x.post()).status, 200);
  const manifest = JSON.parse(Buffer.from(x.bucket.objects.get('check/123/1.json').bytes));
  assert.deepEqual(Object.keys(manifest).sort(), [
    'attempt',
    'batch',
    'box_claim',
    'die',
    'family',
    'format',
    'object',
    'profile',
    'run',
    'scan',
    'scrub',
    'sha256',
    'size',
    'tier',
    'uploaded_at',
    'verdict',
  ]);
  const options = x.bucket.objects.get('check/123/1.tgz').options;
  assert.deepEqual(Object.keys(options).sort(), ['customMetadata', 'httpMetadata']);
  assert.deepEqual(options.customMetadata, { run: '123', attempt: '1', sha256: manifest.sha256 });
  assert.equal(
    x.bucket.objects.get('check/123/1.json').options.httpMetadata.contentType,
    'application/json',
  );
});
test('evidence: the size of an upload at exactly the cap is the size in the answer and the manifest', async () => {
  const x = await harness();
  const edge = new Uint8Array(EVIDENCE_MAX_BYTES);
  edge.set(bundle());
  const r = await x.post(edge);
  assert.equal(r.status, 200);
  assert.equal((await r.json()).size, EVIDENCE_MAX_BYTES);
  const manifest = JSON.parse(Buffer.from(x.bucket.objects.get('check/123/1.json').bytes));
  assert.equal(manifest.size, EVIDENCE_MAX_BYTES);
  assert.equal(manifest.sha256, sha(edge));
});
test('evidence: a bucket that never answers is a 503 after the bound, and a later try can store', async () => {
  const x = await harness();
  evidenceTimeouts.r2 = 30;
  try {
    x.bucket.put = () => new Promise(() => {});
    assert.equal((await x.post()).status, 503);
    assert.equal((await x.h.storage.get('record:' + run)).evidence, undefined);
  } finally {
    evidenceTimeouts.r2 = 20000;
  }
  const real = new Bucket();
  x.bucket.put = real.put.bind(real);
  assert.equal((await x.post()).status, 200);
});
test('evidence: a manifest write that never answers does not fail the upload', async () => {
  const x = await harness();
  const put = x.bucket.put.bind(x.bucket);
  x.bucket.put = (key, ...rest) =>
    key.endsWith('.json') ? new Promise(() => {}) : put(key, ...rest);
  evidenceTimeouts.r2 = 30;
  try {
    assert.equal((await x.post()).status, 200);
  } finally {
    evidenceTimeouts.r2 = 20000;
  }
  assert.deepEqual([...x.bucket.objects.keys()], ['check/123/1.tgz']);
});
test('evidence: the once-per-rental flag is the ledger, not the bucket', async () => {
  const x = await harness();
  assert.equal((await x.post(bundle('first'))).status, 200);
  const puts = x.bucket.puts;
  x.bucket.objects.clear();
  x.h.time.value += 600000;
  x.h.time.value -= 600000; // the clock stays inside the run; only the bucket forgot
  const again = await x.post(bundle('second'));
  assert.equal(again.status, 409);
  assert.equal(x.bucket.puts, puts, 'a refused upload never reaches the bucket');
  const r = await x.h.storage.get('record:' + run);
  assert.equal(r.evidence.state, 'STORED');
});
test('evidence: two uploads at once store one', async () => {
  const x = await harness();
  let release;
  const gate = new Promise((resolve) => (release = resolve));
  const put = x.bucket.put.bind(x.bucket);
  x.bucket.put = async (key, ...rest) => {
    await gate;
    return put(key, ...rest);
  };
  const first = x.post(bundle('first'));
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  const second = await x.post(bundle('second'));
  assert.equal(second.status, 409);
  release();
  assert.equal((await first).status, 200);
  assert.equal(
    Buffer.from(x.bucket.objects.get('check/123/1.tgz').bytes).length,
    bundle('first').length,
  );
});
test('evidence: a fresh PENDING flag blocks, a stale one (a crashed attempt) does not', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:' + run);
  r.evidence = { state: 'PENDING', at: x.h.time.value - 1000 };
  await x.h.storage.put('record:' + run, r);
  assert.equal((await x.post()).status, 409);
  r.evidence = { state: 'PENDING', at: x.h.time.value - 61000 };
  await x.h.storage.put('record:' + run, r);
  assert.equal((await x.post()).status, 200);
  assert.equal((await x.h.storage.get('record:' + run)).evidence.state, 'STORED');
});
test('evidence: a PENDING flag blocks for a minute and not a second more', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:' + run);
  r.evidence = { state: 'PENDING', at: x.h.time.value - 59999 };
  await x.h.storage.put('record:' + run, r);
  const pending = await x.post();
  assert.equal(pending.status, 409);
  assert.equal((await pending.json()).state, 'EVIDENCE_PENDING');
  r.evidence = { state: 'PENDING', at: x.h.time.value - 60000 };
  await x.h.storage.put('record:' + run, r);
  assert.equal((await x.post()).status, 200);
});
test('evidence: an object that is already there stays a conflict, with its state named', async () => {
  const x = await harness();
  x.bucket.objects.set('check/123/1.tgz', { bytes: new Uint8Array(1), options: {} });
  const first = await x.post();
  assert.equal(first.status, 409);
  assert.equal((await first.json()).state, 'EVIDENCE_EXISTS');
  x.bucket.objects.clear();
  const again = await x.post();
  assert.equal(again.status, 409, 'the stored flag outlives the object');
});
test('evidence: a failed write clears the flag so the box can try again', async () => {
  const x = await harness();
  x.bucket.fail = true;
  assert.equal((await x.post()).status, 503);
  assert.equal((await x.h.storage.get('record:' + run)).evidence, undefined);
  x.bucket.fail = false;
  assert.equal((await x.post()).status, 200);
});
test('evidence: an object already in the bucket is a conflict and counts as stored', async () => {
  const x = await harness();
  x.bucket.objects.set('check/123/1.tgz', { bytes: new Uint8Array(1), options: {} });
  const r = await x.post();
  assert.equal(r.status, 409);
  assert.equal(x.bucket.puts, 0);
  assert.equal((await x.h.storage.get('record:' + run)).evidence.state, 'STORED');
});
test('evidence: a failing head check is a 503 and clears the flag', async () => {
  const x = await harness();
  x.bucket.head = async () => {
    throw new Error('R2 down');
  };
  assert.equal((await x.post()).status, 503);
  assert.equal(x.bucket.puts, 0);
  assert.equal((await x.h.storage.get('record:' + run)).evidence, undefined);
});
// --- the gateway holds the body; the ledger only reserves and settles ---------------------------
const stream = (counter, chunk = 1024) =>
  new ReadableStream(
    {
      pull(controller) {
        counter.pulled++;
        controller.enqueue(new Uint8Array(chunk));
      },
    },
    { highWaterMark: 0 },
  );
const streamed = (x, length, headers = {}, t = x.token) => {
  const counter = { pulled: 0 };
  return {
    counter,
    response: x.raw('/evidence/' + run, {
      method: 'POST',
      duplex: 'half',
      headers: { ...(t ? { Authorization: 'Bearer ' + t } : {}), ...headers },
      body: stream(counter),
    }),
  };
};
test('evidence gateway: a bad token never reaches the ledger or reads a byte', async () => {
  const x = await harness();
  const s = streamed(x, 1000, { 'Content-Length': '1000' }, 'bad');
  assert.equal((await s.response).status, 401);
  assert.deepEqual(x.internal, []);
  assert.equal(s.counter.pulled, 0);
});
test('evidence gateway: a length above the cap is 413 and the body is not read', async () => {
  const x = await harness();
  const s = streamed(x, 0, { 'Content-Length': String(EVIDENCE_MAX_BYTES + 1) });
  assert.equal((await s.response).status, 413);
  assert.deepEqual(x.internal, []);
  assert.equal(s.counter.pulled, 0);
});
test('evidence gateway: no declared length is 411 before the body is read', async () => {
  const x = await harness();
  const s = streamed(x, 0, {});
  assert.equal((await s.response).status, 411);
  assert.deepEqual(x.internal, []);
  assert.equal(s.counter.pulled, 0);
});
test('evidence gateway: a refused reservation (409, 404, 401) never reads the body', async () => {
  const x = await harness();
  assert.equal((await x.post()).status, 200);
  const again = streamed(x, 1000, { 'Content-Length': '1000' });
  assert.equal((await again.response).status, 409);
  assert.equal(again.counter.pulled, 0);
});
test('evidence gateway: a valid body far above the report limit is stored whole, and the ledger never sees it', async () => {
  const x = await harness();
  const body = new Uint8Array(3 * 1024 * 1024);
  body.set(bundle());
  const r = await x.post(body);
  assert.equal(r.status, 200);
  assert.equal(Buffer.from(x.bucket.objects.get('check/123/1.tgz').bytes).length, body.length);
  assert.deepEqual(x.internal, ['/evidence/reserve', '/evidence/settle']);
  assert.ok(
    x.sizes.every((n) => n < 1024),
    'only a few bytes go to the ledger: ' + x.sizes,
  );
  assert.deepEqual(x.forwarded, [], 'no external request is forwarded to the ledger');
});
test('evidence gateway: a body longer than it declared is 413 and the flag is released', async () => {
  const x = await harness();
  const r = await x.post(new Uint8Array(EVIDENCE_MAX_BYTES + 4096), {
    headers: { 'Content-Length': '100' },
    digest: 'a'.repeat(64),
  });
  assert.equal(r.status, 413);
  assert.equal(x.bucket.puts, 0);
  assert.equal((await x.h.storage.get('record:' + run)).evidence, undefined);
  assert.equal((await x.post()).status, 200);
});
test('evidence gateway: at most six uploads a minute per rental, then 429 before the ledger', async () => {
  const x = await harness();
  const answers = [];
  for (let i = 0; i < 8; i++) answers.push((await x.post()).status);
  assert.deepEqual(answers, [200, 409, 409, 409, 409, 409, 429, 429]);
  const calls = x.internal.length;
  assert.equal((await x.post()).status, 429);
  assert.equal(x.internal.length, calls);
});
test('evidence gateway: the six-a-minute window reopens after a minute', async () => {
  const x = await harness();
  for (let i = 0; i < 7; i++) await x.post();
  assert.equal((await x.post()).status, 429);
  x.h.time.value += 61000;
  assert.equal((await x.post()).status, 409);
});
test('evidence gateway: at most two uploads are held in one isolate, the third is 503', async () => {
  const x = await harness();
  let release;
  x.hold.reserve = new Promise((resolve) => (release = resolve));
  const first = x.post();
  const second = x.post();
  await new Promise((resolve) => setImmediate(resolve));
  // Integration 2026-11-06: never await an answer that a missing cap would hold forever (it waits
  // on the held reservation). Race it against a short timer, release the hold BEFORE asserting,
  // and settle everything, so a mutant fails this test cleanly instead of hanging the file.
  const pending = Symbol('pending');
  let timer;
  const third = await Promise.race([
    x.post(),
    new Promise((resolve) => (timer = setTimeout(() => resolve(pending), 2000))),
  ]);
  clearTimeout(timer);
  release();
  const done = [(await first).status, (await second).status].sort();
  assert.notEqual(third, pending, 'the third upload waited for the held ones instead of a 503');
  assert.equal(third.status, 503);
  assert.equal((await third.json()).state, 'EVIDENCE_BUSY');
  assert.deepEqual(done, [200, 409]);
  assert.equal((await x.post()).status, 409, 'the count is released');
});
// Integration 2026-11-06: a second, counter-based check of the in-isolate cap that cannot hang.
test('evidence gateway: a third upload while two are held never reaches the ledger', async () => {
  const x = await harness();
  let release;
  x.hold.reserve = new Promise((resolve) => (release = resolve));
  const reserves = () => x.internal.filter((p) => p === '/evidence/reserve').length;
  const settle = async (ms) => {
    for (const end = Date.now() + ms; Date.now() < end && reserves() < 3; )
      await new Promise((resolve) => setTimeout(resolve, 5));
  };
  const uploads = [x.post(), x.post()];
  await settle(1000); // both reach the held reservation (stops early at 3 calls)
  const before = reserves();
  uploads.push(x.post());
  await settle(300);
  const after = reserves();
  release();
  await Promise.allSettled(uploads);
  assert.equal(before, 2, 'the two held uploads reserved');
  assert.equal(after, 2, 'the third was refused before any ledger call');
});
test('evidence gateway: the held count is released when the write fails', async () => {
  const x = await harness();
  x.bucket.fail = true;
  for (let i = 0; i < 3; i++) assert.equal((await x.post()).status, 503);
  x.bucket.fail = false;
  assert.equal((await x.post()).status, 200);
});
// --- the ledger steps ----------------------------------------------------------------------------
async function more(x, n) {
  const base = await x.h.storage.get('record:' + run);
  const out = [];
  for (let i = 0; i < n; i++) {
    const key = String(200 + i) + '-1';
    const rec = { ...base, key, run: String(200 + i), label: 'kf-ci-' + key };
    await x.h.storage.put('record:' + key, rec);
    out.push({ key, token: callbackToken(rec) });
  }
  return out;
}
test('evidence ledger: at most four reservations are open across all rentals', async () => {
  const x = await harness();
  const rentals = await more(x, 5);
  const ok = [];
  for (const r of rentals.slice(0, 4))
    ok.push(await x.reserve({ key: r.key, length: 10 }, r.token));
  assert.equal(ok.length, 4);
  await assert.rejects(x.reserve({ key: rentals[4].key, length: 10 }, rentals[4].token), (e) => {
    assert.equal(e.status, 503);
    assert.equal(e.state, 'EVIDENCE_BUSY');
    return true;
  });
  await x.settle({ key: rentals[0].key, at: ok[0].at, state: 'FAILED' });
  assert.equal(
    (await x.reserve({ key: rentals[4].key, length: 10 }, rentals[4].token)).state,
    'RESERVED',
  );
});
const viaGateway = (x, r) =>
  x.post(bundle('from ' + r.key), { t: r.token, path: '/evidence/' + r.key });
test('evidence: the fifth rental to upload at once is told to retry (503), through the gateway', async () => {
  const x = await harness();
  const rentals = await more(x, 5);
  const open = [];
  for (const r of rentals.slice(0, 4))
    open.push(await x.reserve({ key: r.key, length: 10 }, r.token));
  const busy = await viaGateway(x, rentals[4]);
  assert.equal(busy.status, 503);
  assert.equal((await busy.json()).state, 'EVIDENCE_BUSY');
  assert.equal(x.bucket.puts, 0);
  await x.settle({ key: rentals[0].key, at: open[0].at, state: 'FAILED' });
  assert.equal((await viaGateway(x, rentals[4])).status, 200);
});
test('evidence: stale reservations never block a later upload, through the gateway', async () => {
  const x = await harness();
  const rentals = await more(x, 5);
  for (const r of rentals.slice(0, 4)) await x.reserve({ key: r.key, length: 10 }, r.token);
  x.h.time.value += 60000;
  assert.equal((await viaGateway(x, rentals[4])).status, 200);
});
test('evidence: finished uploads release their reservation, so any number can follow', async () => {
  const x = await harness();
  const rentals = await more(x, 6);
  for (const r of rentals) assert.equal((await viaGateway(x, r)).status, 200);
  assert.equal(x.bucket.objects.size, 12);
});
test('evidence ledger: a reservation that was never settled stops counting after a minute', async () => {
  const x = await harness();
  const rentals = await more(x, 5);
  for (const r of rentals.slice(0, 4)) await x.reserve({ key: r.key, length: 10 }, r.token);
  x.h.time.value += 60000;
  assert.equal(
    (await x.reserve({ key: rentals[4].key, length: 10 }, rentals[4].token)).state,
    'RESERVED',
  );
});
test('evidence ledger: reserve answers keys and rental facts, never the token or the box text', async () => {
  const x = await harness();
  const got = await x.reserve();
  assert.equal(got.state, 'RESERVED');
  assert.equal(got.keys.object, 'check/123/1.tgz');
  assert.deepEqual(Object.keys(got.facts).sort(), ['batch', 'die', 'family', 'tier', 'verdict']);
  assert.ok(!JSON.stringify(got).includes(x.token));
});
test('evidence ledger: reserve checks the token, the length and the request shape', async () => {
  const x = await harness();
  const status = async (body, t) => {
    try {
      await x.reserve(body, t);
      return 200;
    } catch (e) {
      return e.status;
    }
  };
  assert.equal(await status({ key: run, length: 10 }, 'bad'), 401);
  assert.equal(await status({ key: '124-1', length: 10 }), 401);
  assert.equal(await status({ key: run, length: EVIDENCE_MAX_BYTES + 1 }), 413);
  assert.equal(await status({ key: run, length: 0 }), 413);
  assert.equal(await status({ key: '../x', length: 10 }), 400);
  assert.equal(await status({ key: run }), 413);
  assert.equal(await status({ key: run, length: 10 }), 200);
});
test('evidence ledger: a token with the right secret but another nonce is refused', async () => {
  const x = await harness();
  const live = await x.h.storage.get('record:' + run);
  await assert.rejects(
    x.reserve({ key: run, length: 10 }, callbackToken(live, undefined, { nonce: 'c'.repeat(32) })),
    (e) => e.status === 401,
  );
  await assert.rejects(
    x.reserve({ key: run, length: 10 }, callbackToken(live, undefined, { sha: 'd'.repeat(40) })),
    (e) => e.status === 401,
  );
});
test('evidence ledger: settling does not give a rental a fresh request budget', async () => {
  const x = await harness();
  let last = 0;
  for (let i = 0; i < 21; i++) {
    try {
      const got = await x.reserve();
      await x.settle({ key: run, at: got.at, state: 'FAILED' });
      last = 200;
    } catch (e) {
      last = e.status;
    }
  }
  assert.equal(last, 429);
});
test('evidence ledger: settle ignores another attempt, validates its input and clears or stores', async () => {
  const x = await harness();
  const got = await x.reserve();
  await x.settle({ key: run, at: got.at + 1, state: 'FAILED' });
  assert.equal((await x.h.storage.get('record:' + run)).evidence.state, 'PENDING');
  for (const bad of [
    { key: run, at: got.at, state: 'NOPE' },
    { key: run, at: 'x', state: 'FAILED' },
    { key: run, at: got.at, state: 'STORED' },
    { key: run, at: got.at, state: 'STORED', sha256: 'zz' },
    { key: '../x', at: got.at, state: 'FAILED' },
  ])
    await assert.rejects(x.settle(bad));
  await x.settle({ key: run, at: got.at, state: 'FAILED' });
  assert.equal((await x.h.storage.get('record:' + run)).evidence, undefined);
  const again = await x.reserve();
  await x.settle({ key: run, at: again.at, state: 'STORED', sha256: 'a'.repeat(64) });
  assert.equal((await x.h.storage.get('record:' + run)).evidence.state, 'STORED');
});
test('evidence: a replacement rental after a provider fault has its own object key', async () => {
  const x = await harness();
  assert.equal((await x.post(bundle('faulted box'))).status, 200);
  const r = await x.h.storage.get('record:' + run);
  delete r.evidence;
  r.providerRetries = 1;
  await x.h.storage.put('record:' + run, r);
  const second = await x.post(bundle('replacement box'));
  assert.equal(second.status, 200);
  assert.deepEqual([...x.bucket.objects.keys()].sort(), [
    'check/123/1-r1.json',
    'check/123/1-r1.tgz',
    'check/123/1.json',
    'check/123/1.tgz',
  ]);
});
test('evidence: the replacement key has the -r1 suffix and no other leg exists', () => {
  assert.equal(
    evidenceKeys({ key: '7-3', profile: 'sweep', providerRetries: 1 }).object,
    'sweep/7/3-r1.tgz',
  );
  assert.equal(
    evidenceKeys({ key: '7-3', profile: 'sweep', providerRetries: 1 }).manifest,
    'sweep/7/3-r1.json',
  );
  assert.equal(evidenceKeys({ key: '7-3', profile: 'sweep' }).object, 'sweep/7/3.tgz');
  assert.throws(() => evidenceKeys({ key: '7-3', profile: 'sweep', providerRetries: 2 }));
});
test('evidence: a busy ledger queue answers 409 BUSY, which is not an EVIDENCE_EXISTS', async () => {
  const x = await harness();
  const answers = await Promise.all(Array.from({ length: 6 }, () => x.reserve().catch((e) => e)));
  const states = answers.map((a) => a.state ?? a.status);
  assert.ok(states.includes('RESERVED'));
  assert.ok(states.includes('BUSY'), String(states));
  assert.ok(
    // Review R1: the others see the first one's PENDING flag, never EVIDENCE_EXISTS.
    states.every((s) => ['RESERVED', 'EVIDENCE_PENDING', 'BUSY'].includes(s)),
    String(states),
  );
});

// Review of #274, R1 (integration 2026-11-06). FALSIFIER: a box whose first upload is still being
// written (PENDING) hears EVIDENCE_EXISTS, which it reports as stored although nothing may arrive.
test('evidence: PENDING answers EVIDENCE_PENDING; only STORED answers EVIDENCE_EXISTS', async () => {
  const x = await harness();
  const r = await x.h.storage.get('record:' + run);
  r.evidence = { state: 'PENDING', at: x.h.time.value };
  await x.h.storage.put('record:' + run, r);
  const pending = await x.post();
  assert.deepEqual([pending.status, (await pending.json()).state], [409, 'EVIDENCE_PENDING']);
  assert.equal(x.bucket.objects.size, 0, 'nothing written while another upload is pending');
  r.evidence = { state: 'STORED', at: x.h.time.value, sha256: 'a'.repeat(64) };
  await x.h.storage.put('record:' + run, r);
  const stored = await x.post();
  assert.deepEqual([stored.status, (await stored.json()).state], [409, 'EVIDENCE_EXISTS']);
});
