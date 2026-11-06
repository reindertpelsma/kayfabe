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
async function harness(extra = {}, { bucket = new Bucket() } = {}) {
  const h = await setup(extra.noBucket ? {} : { EVIDENCE_BUCKET: bucket, ...extra });
  await h.call();
  const live = await h.storage.get('record:' + run);
  live.bootDeadline = h.time.value + 3600000;
  live.hardDeadline = h.time.value + 7200000;
  await h.storage.put('record:' + run, live);
  const token = callbackToken(live);
  return {
    h,
    bucket,
    token,
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
      return h.service.fetch(
        new Request(h.bindings.WORKER_URL + path, { method, headers: h2, body }),
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
  const noLength = await x.h.service.fetch(
    new Request(x.h.bindings.WORKER_URL + '/evidence/' + run, {
      method: 'POST',
      headers: {
        Authorization: 'Bearer ' + x.token,
        'X-KF-Evidence-SHA256': sha(bundle()),
        'X-KF-Evidence-Meta': JSON.stringify(META),
        'Content-Length': 'abc',
      },
      body: bundle(),
    }),
  );
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
});
test('evidence: disabled mode answers 404 before it reads or checks anything about the body', async () => {
  const x = await harness({ noBucket: true });
  const r = await x.post(bundle(), { length: 9 * 1024 * 1024 });
  assert.equal(r.status, 404);
  assert.equal((await r.json()).state, 'EVIDENCE_DISABLED');
  assert.equal((await x.post(Buffer.from('plain'), { digest: 'zz', meta: null })).status, 404);
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
test('evidence: the request limit answers 429 and the window reopens', async () => {
  const x = await harness();
  for (let i = 0; i < 20; i++) await x.post();
  assert.equal((await x.post()).status, 429);
  x.h.time.value += 10000;
  assert.equal((await x.post()).status, 409);
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
  assert.equal((await x.post()).status, 409);
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
// --- gateway (the stateless front, before the Durable Object) ---------------------------------
function gatewayHarness() {
  let entered = 0,
    seen = null;
  const clock = { value: 1800000000000 };
  const gateway = createGateway({ now: () => clock.value });
  const bindings = {
    ...env,
    BROKER_PATH_PREFIX: 'g'.repeat(32),
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async (request) => {
          entered++;
          seen = {
            length: request.headers.get('Content-Length'),
            bytes: request.body ? (await request.arrayBuffer()).byteLength : 0,
          };
          return Response.json({ state: 'STORED' });
        },
      }),
    },
  };
  const live = {
    run: '123',
    attempt: '1',
    sha: 'a'.repeat(40),
    nonce: 'b'.repeat(32),
    reportExpiry: 1800000000 + 3600,
  };
  const token = callbackToken(live);
  return {
    clock,
    get entered() {
      return entered;
    },
    get seen() {
      return seen;
    },
    post(body, { t = token, headers = {}, path = '/evidence/123-1' } = {}) {
      return gateway.fetch(
        new Request(env.WORKER_URL + '/' + 'g'.repeat(32) + path, {
          method: 'POST',
          headers: { ...(t ? { Authorization: 'Bearer ' + t } : {}), ...headers },
          body,
          ...(body instanceof ReadableStream ? { duplex: 'half' } : {}),
        }),
        bindings,
      );
    },
  };
}
test('evidence gateway: a bad token never reaches the Durable Object or buffers a byte', async () => {
  const g = gatewayHarness();
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
  const r = await g.post(stream, { t: 'bad', headers: { 'Content-Length': '1000' } });
  assert.equal(r.status, 401);
  assert.equal(g.entered, 0);
  assert.equal(pulled, 0);
});
test('evidence gateway: a length above the cap is 413 and the body is not read', async () => {
  const g = gatewayHarness();
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
  const r = await g.post(stream, {
    headers: { 'Content-Length': String(EVIDENCE_MAX_BYTES + 1) },
  });
  assert.equal(r.status, 413);
  assert.equal(g.entered, 0);
  assert.equal(pulled, 0);
});
test('evidence gateway: no declared length is 411 before the body is read', async () => {
  const g = gatewayHarness();
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
  const r = await g.post(stream, { headers: {} });
  assert.equal(r.status, 411);
  assert.equal(g.entered, 0);
  assert.equal(pulled, 0);
});
test('evidence gateway: a body longer than it declared is cut at the cap plus one byte', async () => {
  const g = gatewayHarness();
  const r = await g.post(new Uint8Array(EVIDENCE_MAX_BYTES + 4096), {
    headers: { 'Content-Length': '100' },
  });
  assert.equal(r.status, 200);
  assert.equal(g.seen.bytes, EVIDENCE_MAX_BYTES + 1);
  assert.equal(g.seen.length, String(EVIDENCE_MAX_BYTES + 1));
});
test('evidence gateway: a valid token and length reaches the Durable Object with the whole body', async () => {
  const g = gatewayHarness();
  const body = bundle('x'.repeat(5000));
  const r = await g.post(body, { headers: { 'Content-Length': String(body.length) } });
  assert.equal(r.status, 200);
  assert.equal(g.entered, 1);
  assert.equal(g.seen.bytes, body.length);
});
test('evidence gateway: a valid body far above the report limit arrives whole', async () => {
  const g = gatewayHarness();
  const body = new Uint8Array(3 * 1024 * 1024);
  body.set(bundle());
  const r = await g.post(body, { headers: { 'Content-Length': String(body.length) } });
  assert.equal(r.status, 200);
  assert.equal(g.seen.bytes, body.length);
  assert.equal(g.seen.length, String(body.length));
});
test('evidence gateway: at most six uploads a minute per rental, then 429 before the Durable Object', async () => {
  const g = gatewayHarness();
  const body = bundle('x');
  const answers = [];
  for (let i = 0; i < 8; i++)
    answers.push(
      (await g.post(body, { headers: { 'Content-Length': String(body.length) } })).status,
    );
  assert.deepEqual(answers, [200, 200, 200, 200, 200, 200, 429, 429]);
  assert.equal(g.entered, 6);
});
test('evidence gateway: the six-a-minute window reopens after a minute', async () => {
  const g = gatewayHarness();
  const body = bundle('x');
  const send = async () =>
    (await g.post(body, { headers: { 'Content-Length': String(body.length) } })).status;
  for (let i = 0; i < 6; i++) assert.equal(await send(), 200);
  assert.equal(await send(), 429);
  g.clock.value += 61000;
  assert.equal(await send(), 200);
  assert.equal(g.entered, 7);
});
