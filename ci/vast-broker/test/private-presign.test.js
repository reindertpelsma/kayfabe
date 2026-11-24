import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setup, callbackToken } from './helpers.js';
import {
  PRIVATE_OBJECTS,
  privateAccess,
  PRIVATE_PATH,
  PRIVATE_REFUSALS,
  privateConfigProblems,
  privateKey,
  privateExpiry,
} from '../src/privatedata.js';
import { BENCH_OBJECTS } from '../src/benchdata.js';
import { PROPIMAGES } from '../src/propimages.js';
// FALSIFIER: a run token WITHOUT the private scope (untrusted provenance, a token issued before the scope existed, a forged claim), a run of the
// wrong profile, a path outside the private prefixes, an object that does not exist, a run past its URL limit, a dead run or a bad token
// receives a Location; or any response, log line or ledger field carries a presigned URL or the name of an object the caller did not ask
// for; or a URL lives past 12 hours or past the run's hard deadline; or the public allow-list changes.
const account = '0123456789abcdef0123456789abcdef',
  accessKey = 'f'.repeat(32),
  secretKey = 'e'.repeat(64),
  imageKey = 'windows-proprietary/kfimg-20261112/image-v2.zip',
  route = (key = imageKey, run = '123-1') => '/bench-data/' + run + '/private/' + key;
async function harness({ source = 'admin', scopes, extra = {} } = {}) {
  const h = await setup({
    R2_ACCOUNT_ID: account,
    R2_BUCKET: 'kayfabe-bench-enam',
    R2_READ_ACCESS_KEY_ID: accessKey,
    R2_READ_SECRET_ACCESS_KEY: secretKey,
    ...extra,
  });
  await h.call();
  const live = await h.storage.get('record:123-1');
  live.bootDeadline = h.time.value + 3600000;
  live.hardDeadline = h.time.value + 7200000;
  live.source = source; // the harness rental is a github check run; the allow rule is on profile or source
  if (scopes !== undefined) live.scopes = scopes;
  await h.storage.put('record:123-1', live);
  h.cloud.r2Keys.add(imageKey);
  h.cloud.r2Keys.add('windows-proprietary/kfimg-20261116/image.zip');
  const token = callbackToken(live);
  const x = {
    h,
    token,
    async patch(fn) {
      const r = await h.storage.get('record:123-1');
      fn(r);
      await h.storage.put('record:123-1', r);
    },
    async get(path = route(), { t = token, method = 'GET' } = {}) {
      return h.service.fetch(
        new Request(h.bindings.WORKER_URL + path, {
          method,
          headers: t === null ? {} : { Authorization: 'Bearer ' + t },
        }),
      );
    },
  };
  return x;
}
const reason = async (r) => (await r.clone().json()).reason;
const everything = async (r) =>
  JSON.stringify([...r.headers]) + (await r.clone().text()) + (r.headers.get('Location') ?? '');

// ---------------------------------------------------------------- the configuration
test('private config: the shipped file is a good configuration', () => {
  assert.deepEqual(privateConfigProblems(PRIVATE_OBJECTS), []);
  assert.deepEqual(
    PRIVATE_OBJECTS.prefixes.map((p) => p.prefix),
    ['windows-proprietary', 'linux-proprietary', 'media-cache', 'specworkstation-4.0.0'],
  );
  assert.equal(PRIVATE_OBJECTS.urls_per_run, 24);
  assert.equal(PRIVATE_OBJECTS.max_ttl_seconds, 43200);
});
test('private config: every object propimages.json names lies under a private prefix, none is in the public allow-list', () => {
  const prefixes = new Set(PRIVATE_OBJECTS.prefixes.map((p) => p.prefix));
  const publicKeys = new Set(BENCH_OBJECTS.values());
  for (const o of PROPIMAGES.objects) {
    assert.ok(prefixes.has(o.object.split('/')[0]), o.object);
    assert.equal(publicKeys.has(o.object), false, o.object);
  }
  for (const key of publicKeys) assert.equal(prefixes.has(key.split('/')[0]), false, key);
});
const edit = (fn) => {
  const c = structuredClone(PRIVATE_OBJECTS);
  fn(c);
  return c;
};
for (const [label, fn, problem] of [
  ['no prefixes', (c) => (c.prefixes = []), 'PREFIXES'],
  [
    'a duplicate prefix',
    (c) => c.prefixes.push({ ...c.prefixes[0] }),
    'DUPLICATE_PREFIX:windows-proprietary',
  ],
  ['a prefix with a slash', (c) => (c.prefixes[0].prefix = 'a/b'), 'PREFIX:a/b'],
  ['a TTL above 12 hours', (c) => (c.max_ttl_seconds = 43201), 'MAX_TTL'],
  ['a minimum above the maximum', (c) => (c.min_ttl_seconds = 50000), 'MIN_TTL'],
  ['no per-run limit', (c) => (c.urls_per_run = 0), 'URLS_PER_RUN'],
  ['the sweep profile allowed', (c) => c.allow.push({ profile: 'sweep', why: 'x' }), 'ALLOW_ROW'],
  [
    'a pull request gate allowed',
    (c) => c.allow.push({ source: 'github', profile: 'check', why: 'x' }),
    'ALLOW_GITHUB',
  ],
  ['a rule that allows everything', (c) => c.allow.push({ why: 'x' }), 'ALLOW_ROW'],
  ['an unknown key', (c) => (c.object_names = []), 'SCHEMA'],
]) {
  test('private config refused: ' + label, () => {
    assert.ok(privateConfigProblems(edit(fn)).includes(problem), problem);
  });
}
test('private key: only a listed prefix, never a dot segment, never another namespace', () => {
  assert.equal(privateKey('windows-proprietary', 'kfimg-20261112/image-v2.zip'), imageKey);
  assert.equal(privateKey('specworkstation-4.0.0', 'a.zip'), 'specworkstation-4.0.0/a.zip');
  for (const [prefix, path, status, why] of [
    ['specviewperf-15.0.0', 'x', 403, 'PREFIX_NOT_PRIVATE'],
    ['winapps-open', 'kfapps.iso', 403, 'PREFIX_NOT_PRIVATE'],
    ['windows-proprietary2', 'x', 403, 'PREFIX_NOT_PRIVATE'],
    ['Windows-Proprietary', 'x', 403, 'PREFIX_NOT_PRIVATE'],
    ['windows-proprietary', '../x', 404, 'OBJECT_MISSING'],
    ['windows-proprietary', 'a/./b', 404, 'OBJECT_MISSING'],
    ['windows-proprietary', 'a/../../b', 404, 'OBJECT_MISSING'],
  ])
    assert.throws(
      () => privateKey(prefix, path),
      (e) => e.status === status && e.reason === why,
      prefix + '/' + path,
    );
});
test('private path: private/<prefix>/<1 to 5 segments>, a strict shape', () => {
  const ok = (p) => PRIVATE_PATH.test('/bench-data/123-1/private/' + p);
  assert.ok(ok('windows-proprietary/a'));
  assert.ok(ok('windows-proprietary/a/b/c/d/e'));
  for (const p of [
    'windows-proprietary',
    'windows-proprietary/',
    'windows-proprietary/a/b/c/d/e/f',
    'windows-proprietary//a',
    'windows proprietary/a',
    'a%2fb/c',
    '/a/b',
  ])
    assert.equal(ok(p), false, p);
  assert.equal(PRIVATE_PATH.test('/bench-data/0-1/private/a/b'), false);
});
test('private expiry: 12 hours at most, the run deadline at most, a short remainder refused', () => {
  assert.equal(privateExpiry(0, 100 * 3600 * 1000), 43200);
  assert.equal(privateExpiry(0, 3600 * 1000), 3600);
  assert.throws(
    () => privateExpiry(0, 30 * 1000),
    (e) => e.reason === 'TOKEN_EXPIRED',
  );
});

// ---------------------------------------------------------------- the route
test('private route: a trusted, live run gets a presigned GET URL for any object under a private prefix', async () => {
  const x = await harness();
  const r = await x.get();
  assert.equal(r.status, 302, await r.clone().text());
  assert.equal(r.headers.get('Cache-Control'), 'no-store');
  const url = new URL(r.headers.get('Location'));
  assert.equal(url.host, account + '.r2.cloudflarestorage.com');
  assert.equal(url.pathname, '/kayfabe-bench-enam/' + imageKey);
  const expires = Number(url.searchParams.get('X-Amz-Expires'));
  assert.ok(expires > 7000 && expires <= 7200, String(expires)); // the run's hard deadline (2 h) is nearer than 12 h
  assert.equal(url.searchParams.get('X-Amz-SignedHeaders'), 'host');
  const seen = await everything(r);
  for (const s of [secretKey, x.token]) assert.ok(!seen.includes(s));
  // an object nobody listed anywhere is served as well (no object-name list): the second image
  assert.equal((await x.get(route('windows-proprietary/kfimg-20261116/image.zip'))).status, 302);
});
test('private route: the lifetime is 12 hours when the run lives longer', async () => {
  const x = await harness();
  await x.patch((r) => {
    r.hardDeadline = x.h.time.value + 20 * 3600 * 1000;
    r.reportExpiry = Math.floor(r.hardDeadline / 1000);
  });
  const t = callbackToken(await x.h.storage.get('record:123-1'));
  const r = await x.get(route(), { t });
  assert.equal(r.status, 302);
  assert.equal(new URL(r.headers.get('Location')).searchParams.get('X-Amz-Expires'), '43200');
});
test('private route: the lifetime never outlives the run', async () => {
  const x = await harness();
  await x.patch((r) => (r.hardDeadline = x.h.time.value + 90000));
  const r = await x.get();
  const left = Number(new URL(r.headers.get('Location')).searchParams.get('X-Amz-Expires'));
  assert.ok(left <= 90 && left >= 60, String(left));
});
test('private route: the presign probe is one HEAD and no GET or list reaches the bucket', async () => {
  const x = await harness();
  await x.get();
  const r2 = x.h.cloud.calls.filter((c) => c.url.includes('r2.cloudflarestorage.com'));
  assert.deepEqual(
    r2.map((c) => c.method),
    ['HEAD'],
  );
  assert.equal(new URL(r2[0].url).pathname, '/kayfabe-bench-enam/' + imageKey);
  assert.equal(new URL(r2[0].url).searchParams.get('X-Amz-Expires'), '60');
});
test('private route: GET only', async () => {
  const x = await harness();
  for (const method of ['POST', 'PUT', 'DELETE'])
    assert.notEqual((await x.get(route(), { method })).status, 302);
});

// ---------------------------------------------------------------- named refusals
const CASES = {
  TOKEN_INVALID: async (x) => x.get(route(), { t: 'bad' }),
  TOKEN_EXPIRED: async (x) => {
    await x.patch((r) => (r.closed = 'STOPPED'));
    return x.get();
  },
  TRUST_SCOPE_MISSING: async (x) => {
    await x.patch((r) => (r.scopes = ['public']));
    return x.get();
  },
  TIER_NOT_ALLOWED: async (x) => {
    await x.patch((r) => (r.source = 'github'));
    return x.get();
  },
  PREFIX_NOT_PRIVATE: async (x) => x.get(route('specviewperf-15.0.0/CHECKSUMS.txt')),
  OBJECT_MISSING: async (x) => x.get(route('windows-proprietary/kfimg-99999999/image.zip')),
};
const STATUS = {
  TOKEN_INVALID: 401,
  TOKEN_EXPIRED: 401,
  TRUST_SCOPE_MISSING: 403,
  TIER_NOT_ALLOWED: 403,
  PREFIX_NOT_PRIVATE: 403,
  OBJECT_MISSING: 404,
};
for (const [name, ask] of Object.entries(CASES))
  test('private route refuses with the named reason ' + name, async () => {
    const x = await harness();
    const r = await ask(x);
    assert.equal(r.status, STATUS[name]);
    assert.equal(await reason(r), name);
    assert.equal(r.headers.get('Location'), null);
    // no response ever names an object or a URL
    const seen = await everything(r);
    assert.ok(
      !/windows-proprietary|kfimg|X-Amz|r2\.cloudflarestorage/.test(
        seen.replace(/"reason":"[A-Z_]+"/, ''),
      ),
      seen,
    );
  });
test('private route: the named refusals are exactly the documented set', () => {
  assert.deepEqual(
    [...PRIVATE_REFUSALS].sort(),
    [...Object.keys(CASES), 'PRIVATE_URL_LIMIT'].sort(),
  );
  const refused = readFileSync(new URL('../src/privatedata.js', import.meta.url), 'utf8');
  for (const name of PRIVATE_REFUSALS) assert.ok(refused.includes("'" + name + "'"), name);
});
test('private route: a token without the scope claim (issued before it existed) is the public scope', async () => {
  const x = await harness();
  const record = await x.h.storage.get('record:123-1');
  // the record says private but the token carries no claim
  const old = callbackToken(record, undefined, { scopes: undefined });
  const r = await x.get(route(), { t: old });
  assert.equal(r.status, 403);
  assert.equal(await reason(r), 'TRUST_SCOPE_MISSING');
});
test('private route: a malformed or forged scope claim is the public scope', async () => {
  const x = await harness();
  const record = await x.h.storage.get('record:123-1');
  for (const bad of [['private'], [], ['public', 'admin'], 'private', ['public', 'public']]) {
    const t = callbackToken(record, undefined, { scopes: bad });
    assert.equal(
      await reason(await x.get(route(), { t })),
      'TRUST_SCOPE_MISSING',
      JSON.stringify(bad),
    );
  }
});
test('private route: the record must carry the scope as well (a token alone is not enough)', async () => {
  const x = await harness({ scopes: ['public'] });
  // a token that claims private, on a record that was decided public
  const t = callbackToken(await x.h.storage.get('record:123-1'), undefined, {
    scopes: ['public', 'private'],
  });
  assert.equal(await reason(await x.get(route(), { t })), 'TRUST_SCOPE_MISSING');
  const none = await harness({ scopes: undefined });
  await none.patch((r) => delete r.scopes);
  assert.equal(await reason(await none.get(route())), 'TRUST_SCOPE_MISSING');
});
test('private route: profiles. release and an admin start may, a github check run and the sweep may not', async () => {
  for (const [label, patch, status] of [
    ['admin start', (r) => (r.source = 'admin'), 302],
    ['release', (r) => ((r.source = 'github'), (r.budgetClass = 'release')), 302],
    ['github check', (r) => (r.source = 'github'), 403],
    [
      'github sweep',
      (r) => ((r.source = 'github'), (r.profile = 'sweep'), delete r.budgetClass),
      403,
    ],
    ['no source', (r) => delete r.source, 403],
  ]) {
    const x = await harness();
    await x.patch(patch);
    const r = await x.get();
    assert.equal(r.status, status, label);
    if (status === 403) assert.equal(await reason(r), 'TIER_NOT_ALLOWED', label);
  }
});
test('private route: a token for another run, a missing token and a missing record are TOKEN_INVALID', async () => {
  const x = await harness();
  const other = callbackToken(await x.h.storage.get('record:123-1'), undefined, { run: '124' });
  for (const [t, path] of [
    [other, route()],
    [null, route()],
    [x.token, route(imageKey, '124-1')],
  ]) {
    const r = await x.get(path, { t });
    assert.equal(r.status, 401);
    assert.equal(r.headers.get('Location'), null);
  }
});
test('private route: a run past its deadline, an ended run and a provider that does not report running are refused', async () => {
  for (const [label, fn] of [
    ['ended', (x, r) => (r.ended = x.h.time.value)],
    ['expired', (x, r) => (x.h.time.value = r.hardDeadline)],
  ]) {
    const x = await harness();
    await x.patch((r) => fn(x, r));
    assert.equal(await reason(await x.get()), 'TOKEN_EXPIRED', label);
  }
  const x = await harness();
  x.h.cloud.instances.get('900').actual_status = 'exited';
  assert.equal(await reason(await x.get()), 'TOKEN_EXPIRED');
});
test('private route: a store that is down is 503, not a redirect', async () => {
  const x = await harness();
  x.h.cloud.r2Status = 500;
  const r = await x.get();
  assert.equal(r.status, 503);
  assert.equal(r.headers.get('Location'), null);
  x.h.cloud.r2Status = 403;
  assert.equal((await x.get()).status, 503);
});
test('private route without R2 settings fails closed', async () => {
  const x = await harness({ extra: { R2_BUCKET: '' } });
  const r = await x.get();
  assert.equal(r.status, 503);
  assert.equal(r.headers.get('Location'), null);
});

// ---------------------------------------------------------------- the per-run limit
const mintAll = async (x) => {
  for (let i = 0; i < PRIVATE_OBJECTS.urls_per_run; i++) {
    x.h.time.value += 11000; // stay under the generic request rate
    assert.equal((await x.get()).status, 302, 'mint ' + i);
  }
  x.h.time.value += 11000;
};
test('private route: at most urls_per_run URLs per run, then RATE_LIMITED', async () => {
  const x = await harness();
  await mintAll(x);
  const over = await x.get();
  assert.equal(over.status, 429);
  assert.equal(await reason(over), 'PRIVATE_URL_LIMIT');
  assert.equal((await over.clone().json()).state, 'RATE_LIMITED');
  assert.equal(over.headers.get('Location'), null);
});
test('private route: the URL count is kept in the ledger record, so a restart does not reset it', async () => {
  const x = await harness();
  await mintAll(x);
  x.h.restart();
  x.h.time.value += 11000;
  assert.equal((await x.get()).status, 429);
  assert.equal((await x.h.storage.get('record:123-1')).privateMints, PRIVATE_OBJECTS.urls_per_run);
});
test('private route: refused requests do not consume the limit', async () => {
  const x = await harness();
  for (const path of [route('windows-proprietary/nope.bin'), route('specviewperf-15.0.0/x')])
    for (let i = 0; i < 3; i++) {
      x.h.time.value += 11000;
      await x.get(path);
    }
  assert.equal((await x.h.storage.get('record:123-1')).privateMints ?? 0, 0);
});
test('private route: the public per-object count is separate from the private count', async () => {
  const x = await harness();
  assert.equal((await x.get()).status, 302);
  const r = await x.h.storage.get('record:123-1');
  assert.equal(r.privateMints, 1);
  assert.equal(r.benchMints, undefined);
});

// ---------------------------------------------------------------- logs, storage, listing
async function logsOf(x, ...asks) {
  const lines = [],
    real = console.log;
  console.log = (...a) => lines.push(a.join(' '));
  try {
    for (const ask of asks) await ask();
  } finally {
    console.log = real;
  }
  return lines;
}
test('private route logs object, run and expiry for a mint and the reason for a refusal', async () => {
  const x = await harness();
  const lines = await logsOf(
    x,
    async () => assert.equal((await x.get()).status, 302),
    async () => assert.equal((await x.get(route('specviewperf-15.0.0/x'))).status, 403),
  );
  const events = lines.filter((l) => l.startsWith('{')).map((l) => JSON.parse(l));
  const minted = events.find((e) => e.event === 'private_data_url');
  assert.deepEqual(Object.keys(minted).sort(), ['event', 'expires', 'object', 'run']);
  assert.equal(minted.run, '123-1');
  assert.equal(minted.object, imageKey);
  assert.equal(minted.expires, Math.floor(x.h.time.value / 1000) + 7200);
  const refused = events.find((e) => e.event === 'private_data_refused');
  assert.deepEqual(Object.keys(refused).sort(), ['event', 'prefix', 'reason', 'run']);
  assert.equal(refused.reason, 'PREFIX_NOT_PRIVATE');
});
test('private route never logs a URL, a signature, a key or the token', async () => {
  const x = await harness();
  const lines = await logsOf(
    x,
    () => x.get(),
    () => x.get(route('windows-proprietary/nope.bin')),
    () => x.get(route(), { t: 'bad' }),
  );
  const all = lines.join('\n');
  for (const s of [
    secretKey,
    accessKey,
    x.token,
    'X-Amz-Signature',
    'https://',
    'r2.cloudflarestorage',
  ])
    assert.ok(!all.includes(s), s);
});
test('private route: the URL is not stored in the ledger and no object name is stored', async () => {
  const x = await harness();
  const r = await x.get();
  const sig = new URL(r.headers.get('Location')).searchParams.get('X-Amz-Signature');
  const stored = JSON.stringify([...x.h.storage.data]);
  assert.ok(!stored.includes(sig));
  assert.ok(!stored.includes(secretKey));
  assert.ok(!stored.includes('kfimg-20261112'));
});
for (const path of [
  '/bench-data/123-1/private',
  '/bench-data/123-1/private/',
  '/bench-data/123-1/private/windows-proprietary',
  '/bench-data/123-1/private/windows-proprietary/',
  '/bench-data/123-1/private/windows-proprietary/kfimg-20261112/',
  '/bench-data/123-1/private/windows-proprietary/%2e%2e/x',
  '/bench-data/123-1/private/windows-proprietary/a%2fb',
  '/bench-data/123-1/private/windows-proprietary/image-v2.zip?list-type=2',
  '/bench-data/123-1/private/windows-proprietary/image-v2.zip?prefix=',
]) {
  test('private route has no listing: ' + path, async () => {
    const x = await harness();
    const r = await x.get(path);
    assert.ok([400, 404].includes(r.status), String(r.status));
    assert.equal(r.headers.get('Location'), null);
    assert.ok(!(await everything(r)).includes('kfimg'));
  });
}
test('public allow-list is unchanged: the six names, and a private name is not served by the public route', async () => {
  assert.deepEqual([...BENCH_OBJECTS.values()].sort(), [
    'specviewperf-15.0.0/CHECKSUMS.txt',
    'specviewperf-15.0.0/SPEC_General_License.pdf',
    'specviewperf-15.0.0/original/SPECviewperf-Setup-15.0.0.exe',
    'specviewperf-15.0.0/specviewperf-15.0.0.tar.zst',
    'winapps-open/kfapps.iso',
    'winapps-open/manifest.json',
  ]);
  const x = await harness();
  assert.equal((await x.get('/bench-data/123-1/image-v2.zip')).status, 404);
  assert.equal((await x.get('/bench-data/123-1/windows-proprietary')).status, 404);
});
test('private route through the public entrypoint: a bad token is stopped at the front door with TOKEN_INVALID', async () => {
  const { default: worker } = await import('../src/index.js');
  const x = await harness();
  const prefix = 'x'.repeat(40);
  let reached = 0;
  const bindings = {
    ...x.h.bindings,
    BROKER_PATH_PREFIX: prefix,
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async (req) => {
          reached++;
          const u = new URL(req.url);
          u.pathname = u.pathname.slice(prefix.length + 1);
          return x.h.service.fetch(new Request(u, req));
        },
      }),
    },
  };
  const ask = (t) =>
    worker.fetch(
      new Request(x.h.bindings.WORKER_URL + '/' + prefix + route(), {
        headers: { Authorization: 'Bearer ' + t },
      }),
      bindings,
    );
  assert.equal((await ask(x.token)).status, 302);
  assert.equal(reached, 1);
  const bad = await ask('bad');
  assert.equal(bad.status, 401);
  assert.equal((await bad.json()).reason, 'TOKEN_INVALID');
  assert.equal(reached, 1, 'a bad token never reaches the ledger');
});
test('private route: a dot-only segment is not an object name even when the store would say yes', async () => {
  const x = await harness();
  x.h.cloud.r2Any = true;
  // (a `..` or `.` segment between two names is collapsed by URL parsing before the Worker sees it; a run of dots is not)
  for (const path of ['windows-proprietary/...', 'windows-proprietary/....']) {
    const r = await x.get(route(path));
    assert.notEqual(r.status, 302, path);
    assert.equal(r.headers.get('Location'), null, path);
  }
  x.h.time.value += 11000;
  assert.equal((await x.get(route('windows-proprietary/...'))).status, 404);
  assert.equal(await reason(await x.get(route('windows-proprietary/...'))), 'OBJECT_MISSING');
  assert.equal(
    (await x.get(route('windows-proprietary/real.zip'))).status,
    302,
    'the store saying yes is honoured for a real name',
  );
});
test('private route: a token whose signature is good and whose time has passed is TOKEN_EXPIRED, any other bad token TOKEN_INVALID', async () => {
  const x = await harness();
  const { RunTokens } = await import('../src/tokens.js');
  const tokens = new RunTokens(x.h.bindings.RUN_TOKEN_SECRET, () => x.h.time.value);
  assert.equal(await tokens.expired(x.token), false);
  const r = await x.h.storage.get('record:123-1');
  x.h.time.value = r.reportExpiry * 1000 + 1000;
  assert.equal(await tokens.expired(x.token), true);
  assert.equal(await tokens.expired('bad'), false);
  assert.equal(
    await tokens.expired(x.token.slice(0, -2) + 'AA'),
    false,
    'a forged signature is invalid, not expired',
  );
  assert.equal(await reason(await x.get(route(), { t: x.token })), 'TOKEN_EXPIRED');
  assert.equal(
    await reason(await x.get(route(), { t: x.token.slice(0, -2) + 'AA' })),
    'TOKEN_INVALID',
  );
});
test('private route through the public entrypoint: an expired token is TOKEN_EXPIRED at the front door', async () => {
  const { createGateway } = await import('../src/index.js');
  const x = await harness();
  const r = await x.h.storage.get('record:123-1');
  x.h.time.value = r.reportExpiry * 1000 + 1000;
  const worker = createGateway({ now: () => x.h.time.value });
  const prefix = 'x'.repeat(40);
  const bindings = {
    ...x.h.bindings,
    BROKER_PATH_PREFIX: prefix,
    LEDGER: {
      idFromName: () => 1,
      get: () => ({
        fetch: async () => {
          throw Error('must not reach the ledger');
        },
      }),
    },
  };
  const res = await worker.fetch(
    new Request(x.h.bindings.WORKER_URL + '/' + prefix + route(), {
      headers: { Authorization: 'Bearer ' + x.token },
    }),
    bindings,
  );
  assert.equal(res.status, 401);
  assert.equal((await res.json()).reason, 'TOKEN_EXPIRED');
});

// ---------------------------------------------------------------- the checks of privateAccess itself (a second line behind the gateway)
const live = (extra = {}) => ({
  key: '1-1',
  ended: null,
  closed: null,
  reportExpiry: 2000,
  scopes: ['public', 'private'],
  source: 'admin',
  profile: 'check',
  ...extra,
});
const fieldsOf = (extra = {}) => ({ scopes: ['public', 'private'], ...extra });
const refusedWith = (r, fields, nowMs, name) =>
  assert.throws(
    () => privateAccess(r, fields, { nowMs }),
    (e) => e.reason === name,
    name,
  );
test('privateAccess: a live trusted run passes and returns its URL count', () => {
  assert.equal(privateAccess(live(), fieldsOf(), { nowMs: 1000 }), 0);
  assert.equal(privateAccess(live({ privateMints: 5 }), fieldsOf(), { nowMs: 1000 }), 5);
});
test('privateAccess: an ended run is TOKEN_EXPIRED', () => {
  refusedWith(live({ ended: 5 }), fieldsOf(), 1000, 'TOKEN_EXPIRED');
});
test('privateAccess: a closed run is TOKEN_EXPIRED', () => {
  refusedWith(live({ closed: 'STOPPED' }), fieldsOf(), 1000, 'TOKEN_EXPIRED');
});
test('privateAccess: a run past its report expiry is TOKEN_EXPIRED', () => {
  refusedWith(live(), fieldsOf(), 2000000, 'TOKEN_EXPIRED');
});
test('privateAccess: no record or no token fields is TOKEN_INVALID', () => {
  refusedWith(null, fieldsOf(), 1000, 'TOKEN_INVALID');
  refusedWith(live(), null, 1000, 'TOKEN_INVALID');
});
test('privateAccess: the scope on either side, the profile or source, and the limit', () => {
  refusedWith(live(), fieldsOf({ scopes: ['public'] }), 1000, 'TRUST_SCOPE_MISSING');
  refusedWith(live({ scopes: ['public'] }), fieldsOf(), 1000, 'TRUST_SCOPE_MISSING');
  refusedWith(live({ source: 'github' }), fieldsOf(), 1000, 'TIER_NOT_ALLOWED');
  refusedWith(
    live({ privateMints: PRIVATE_OBJECTS.urls_per_run }),
    fieldsOf(),
    1000,
    'PRIVATE_URL_LIMIT',
  );
  assert.equal(
    privateAccess(
      live({ source: 'github', profile: 'check', budgetClass: 'release' }),
      fieldsOf(),
      { nowMs: 1000 },
    ),
    0,
  );
});
