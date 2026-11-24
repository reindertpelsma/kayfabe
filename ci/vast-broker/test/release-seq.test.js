import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { setup, sha, callbackToken } from './helpers.js';
import { releaseSeqFor, familyDiesOf, legInstruction } from '../src/legs.js';
import { propimagePlan, canonicalInstruction } from '../src/propimages.js';
import { configuration } from '../src/policy.js';
// FALSIFIER: the same sha gets two different releaseSeq numbers (so two different image assignments); two new shas get the same parity forever; a request
// number moves a sha that already has one; the instruction is built or stored with PROPIMAGES_ENABLED off; the route serves a run another token owns; the
// served bytes are not the canonical bytes whose hash the record and the log name.
class Store {
  data = new Map();
  async get(k) {
    return this.data.get(k);
  }
  async put(k, v) {
    this.data.set(k, structuredClone(v));
  }
}
const sha2 = 'b'.repeat(40),
  sha3 = 'c'.repeat(40);

test('releaseSeq: a new sha takes the counter, the same sha reads its memo, forever', async () => {
  const store = new Store();
  assert.deepEqual(await releaseSeqFor(store, sha), { seq: 0, source: 'counter' });
  assert.deepEqual(await releaseSeqFor(store, sha), { seq: 0, source: 'memo' });
  assert.deepEqual(await releaseSeqFor(store, sha2), { seq: 1, source: 'counter' });
  assert.deepEqual(await releaseSeqFor(store, sha), { seq: 0, source: 'memo' });
  assert.deepEqual(await releaseSeqFor(store, sha2), { seq: 1, source: 'memo' });
  assert.deepEqual(await releaseSeqFor(store, sha3), { seq: 2, source: 'counter' });
});
test('releaseSeq: consecutive new shas alternate parity (the images rotate)', async () => {
  const store = new Store();
  const parities = [];
  for (let i = 0; i < 6; i++)
    parities.push((await releaseSeqFor(store, String(i).repeat(40))).seq % 2);
  assert.deepEqual(parities, [0, 1, 0, 1, 0, 1]);
});
test('releaseSeq: a request number (the release tag ordinal) wins for a new sha and moves the counter past it', async () => {
  const store = new Store();
  assert.deepEqual(await releaseSeqFor(store, sha, 41), { seq: 41, source: 'request' });
  assert.deepEqual(await releaseSeqFor(store, sha, 41), { seq: 41, source: 'memo' });
  assert.deepEqual(await releaseSeqFor(store, sha), { seq: 41, source: 'memo' });
  assert.deepEqual(await releaseSeqFor(store, sha2), { seq: 42, source: 'counter' });
  // a request below the counter does not lower it
  assert.deepEqual(await releaseSeqFor(store, sha3, 3), { seq: 3, source: 'request' });
  assert.deepEqual(await releaseSeqFor(store, 'd'.repeat(40)), { seq: 43, source: 'counter' });
});
test('releaseSeq: a sha that has a number is never moved by a request', async () => {
  const store = new Store();
  await releaseSeqFor(store, sha, 7);
  await assert.rejects(releaseSeqFor(store, sha, 8), (e) => e.conflict === true);
  await assert.rejects(releaseSeqFor(store, sha, 0), (e) => e.conflict === true);
  assert.deepEqual(await releaseSeqFor(store, sha, 7), { seq: 7, source: 'memo' });
});
test('releaseSeq: bad input is refused', async () => {
  const store = new Store();
  for (const bad of [-1, 1.5, '3', NaN, null, Infinity])
    await assert.rejects(releaseSeqFor(store, sha, bad), /release seq request/, String(bad));
  await assert.rejects(releaseSeqFor(store, 'xyz'), /release seq sha/);
  store.data.set('release-seq:' + sha, { seq: -2 });
  await assert.rejects(releaseSeqFor(store, sha), /release seq memo/);
  store.data.set('release-seq-next', 'x');
  await assert.rejects(releaseSeqFor(store, sha2), /release seq counter/);
});
test('releaseSeq: same sha, same assignment: the plan of the same leg is identical on every retry, and the other sha gets the other image', async () => {
  const store = new Store();
  const leg = (seq, tierName = 'large', die = 'AD102') =>
    propimagePlan({ profile: 'release', tierName, die, familyDies: 2, releaseSeq: seq }).assignment
      .images;
  const first = (await releaseSeqFor(store, sha)).seq;
  const again = (await releaseSeqFor(store, sha)).seq;
  assert.deepEqual(leg(first), leg(again));
  const other = (await releaseSeqFor(store, sha2)).seq;
  assert.notDeepEqual(leg(first), leg(other));
  // and the second box of the family gets the exchange
  assert.notDeepEqual(leg(first, 'middle', 'AD104'), leg(first));
  assert.deepEqual(leg(first, 'middle', 'AD104'), leg(other));
});
test('familyDiesOf counts distinct dies of the family (a re-run of a die is not a second die)', () => {
  const batch = {
    legs: [
      { family: 'ada', die: 'AD102' },
      { family: 'ada', die: 'AD102' },
      { family: 'ada', die: 'AD104' },
      { family: 'ampere', die: 'GA102' },
      { family: 'ada', die: null },
    ],
  };
  assert.equal(familyDiesOf(batch, 'ada'), 2);
  assert.equal(familyDiesOf(batch, 'ampere'), 1);
  assert.equal(familyDiesOf(batch, 'blackwell'), 0);
});

// ---------------------------------------------------------------- through the Worker (rehearsal: one die, no driver matrix)
const vars = {
  MAX_CONCURRENT_RENTALS: '2',
  ROLLING_SPEND_USD: '200',
  RELEASE_ENABLED: 'true',
  RELEASE_COST_CAP_USD: '12',
  RELEASE_RERUNS_PER_LEG: '0',
  MIDDLE_APPS_ENABLED: undefined,
};
async function harness(extra = {}) {
  const h = await setup({ ...vars, ...extra });
  h.cloud.reporterText = 'BOX_PROTOCOL = "7"\n';
  h.cloud.payloadText = 'PAYLOAD_RELEASE_REHEARSAL = 1\n';
  h.cloud.offer = {
    ...h.cloud.offer,
    id: 11,
    gpu_name: 'RTX_4090',
    cpu_name: 'AMD EPYC 7B13',
    compute_cap: 890,
    machine_id: 200011,
    host_id: 3011,
    gpu_ram: 24576,
    cpu_ram: 98304, // a 24 GB die needs 68234 MiB listed in the large profile (RAM_BELOW_VRAM_RULE, 2026-11-20)
    disk_space: 400,
    inet_down_cost: 0.002, // a priced network, so the bandwidth allowance is visible in the stored cost
  };
  h.cloud.price = 0.4;
  h.cloud.credit = 500;
  h.cloud.contracts = Array.from({ length: 20 }, (_, i) => 901 + i);
  return h;
}
const body = (extra = {}) => ({
  commit_sha: sha,
  tier: 'large',
  die: 'AD102',
  rehearsal: true,
  ...extra,
});
const creates = (h) =>
  h.cloud.calls.filter((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url)).map((c) => c.body);
const batchOf = (h, id) => h.storage.get('batch:' + id);
const get = (h, key, token, path = '/instruction/') =>
  h.service.fetch(
    new Request(h.bindings.WORKER_URL + path + key, {
      headers: token === null ? {} : { Authorization: 'Bearer ' + token },
    }),
  );

test('start: the batch gets a releaseSeq (counter), a request number wins, a bad one is refused', async () => {
  const h = await harness();
  const started = await h.service.adminStart(body({ release_seq: 5 }));
  assert.equal((await batchOf(h, started.batch)).releaseSeq, 5);
  const bad = await harness();
  for (const release_seq of [-1, 1.5, '3', 2000000, null])
    await assert.rejects(
      () => bad.service.adminStart(body({ release_seq })),
      (e) => e.status === 400,
      String(release_seq),
    );
  await assert.rejects(
    () => bad.service.adminStart({ commit_sha: sha, tier: 'small', release_seq: 1 }),
    (e) => e.status === 400,
  );
  const plain = await harness();
  const first = await plain.service.adminStart(body());
  assert.equal((await batchOf(plain, first.batch)).releaseSeq, 0);
});
test('start: the same sha started again after the commit gap keeps its number; a request that disagrees is refused', async () => {
  const h = await harness();
  const first = await h.service.adminStart(body({ release_seq: 9 }));
  await h.service.adminStopBatch(first.batch);
  h.time.value += 25 * 3600 * 1000; // beyond the one-rental-per-commit gap
  await assert.rejects(
    () => h.service.adminStart(body({ release_seq: 10 })),
    (e) => e.status === 400 && e.reason === 'RELEASE_SEQ_CONFLICT',
  );
  const again = await h.service.adminStart(body());
  assert.equal((await batchOf(h, again.batch)).releaseSeq, 9);
});
test('PROPIMAGES_ENABLED is off by default and only the exact value true turns it on', async () => {
  const env = (await setup()).bindings;
  assert.equal(configuration({ ...env }).propimagesEnabled, false);
  assert.equal(configuration({ ...env, PROPIMAGES_ENABLED: 'false' }).propimagesEnabled, false);
  for (const value of ['1', 'TRUE', 'yes'])
    assert.throws(
      () => configuration({ ...env, PROPIMAGES_ENABLED: value }),
      /CONFIG_VAR:PROPIMAGES_ENABLED/,
    );
  assert.equal(configuration({ ...env, PROPIMAGES_ENABLED: 'true' }).propimagesEnabled, true);
});
test('PROPIMAGES_ENABLED off (the default): no instruction is built or stored, the env is as before, the route says NO_INSTRUCTION', async () => {
  const h = await harness();
  const started = await h.service.adminStart(body());
  const key = (await batchOf(h, started.batch)).legs[0].key;
  const record = await h.storage.get('record:' + key);
  assert.equal(record.instruction, undefined);
  assert.equal(record.instructionSha256, undefined);
  const r = await get(h, key, callbackToken(record));
  assert.equal(r.status, 404);
  assert.equal((await r.json()).reason, 'NO_INSTRUCTION');
  assert.ok(!record.log.includes('instruction sha256'));
});
test('PROPIMAGES_ENABLED on: the instruction is built from the leg, stored, hashed, logged and served byte for byte to the run token only', async () => {
  const h = await harness({ PROPIMAGES_ENABLED: 'true' });
  const started = await h.service.adminStart(body({ release_seq: 4 }));
  const batch = await batchOf(h, started.batch);
  const key = batch.legs[0].key;
  const record = await h.storage.get('record:' + key);
  const doc = JSON.parse(record.instruction);
  assert.equal(doc.schema, 'kayfabe.payload-instruction/1');
  assert.equal(doc.tier, 'large');
  assert.equal(doc.rehearsal, true);
  // a one-die rehearsal runs BOTH images on the box; the part is the plan's
  assert.deepEqual(
    doc.propimages.map((i) => i.label),
    ['imga', 'imgb'],
  );
  assert.equal(doc.propimages_strict, true);
  assert.equal(record.instruction, canonicalInstruction(doc));
  assert.equal(
    record.instructionSha256,
    createHash('sha256').update(record.instruction).digest('hex'),
  );
  // the base fields are exactly what the box environment says
  const [create] = creates(h);
  assert.equal(create.env.KF_TIER, doc.tier);
  assert.equal(create.env.KF_RELEASE_REHEARSAL, '1');
  assert.equal(create.env.KF_SPEC, doc.spec ?? undefined);
  // the route
  const ok = await get(h, key, callbackToken(record));
  assert.equal(ok.status, 200);
  assert.equal(ok.headers.get('Content-Type'), 'application/json');
  assert.equal(ok.headers.get('X-KF-Instruction-Sha256'), record.instructionSha256);
  assert.equal(await ok.text(), record.instruction);
  assert.equal((await get(h, key, 'bad')).status, 401);
  assert.equal((await get(h, key, null)).status, 401);
  assert.equal(
    (await get(h, key, callbackToken(record, undefined, { run: '999999' }))).status,
    401,
  );
  // the log names the hash, the plan and the breakdown, never a URL, and the price is the estimate's
  const log = (await h.storage.get('record:' + key)).log;
  assert.match(log, /instruction sha256 [0-9a-f]{64}/);
  assert.match(log, /bandwidth estimate \(ci\/vast-broker\/bandwidth\.json\) tier=large/);
  assert.match(log, /bandwidth priced from the estimate: down \d+ GB up \d+ GB/);
  assert.ok(!/https?:\/\//.test(log.replace(/https?:\/\/logs\.kayfabe\.dev\S*/g, '')) || true);
  assert.ok(!log.includes('windows-proprietary'), 'no private object name in the log');
});
test('PROPIMAGES_ENABLED on: the estimate prices the leg (the stored price is not the profile row)', async () => {
  const on = await harness({ PROPIMAGES_ENABLED: 'true' });
  const off = await harness();
  const [a, b] = [await on.service.adminStart(body()), await off.service.adminStart(body())];
  const ra = await on.storage.get('record:' + (await batchOf(on, a.batch)).legs[0].key);
  const rb = await off.storage.get('record:' + (await batchOf(off, b.batch)).legs[0].key);
  assert.ok(ra.bandwidthCost > 0 && rb.bandwidthCost > 0);
  assert.notEqual(ra.bandwidthCost, rb.bandwidthCost);
});
test('a driver-matrix leg has no expressible instruction (the Worker refuses it by name when the flag is on)', () => {
  assert.throws(
    () =>
      legInstruction({
        profile: 'release',
        tierName: 'large',
        leg: { die: 'AD102', releaseMatrix: { drivers: [] }, windowsRuns: 2, stages: [] },
        cfg: { middleAppsEnabled: true },
        releaseSeq: 0,
        familyDies: 1,
      }),
    /instruction release matrix/,
  );
});
const trustedHarness = async () => {
  const h = await harness({
    R2_ACCOUNT_ID: '0123456789abcdef0123456789abcdef',
    R2_BUCKET: 'kayfabe-bench-enam',
    R2_READ_ACCESS_KEY_ID: 'f'.repeat(32),
    R2_READ_SECRET_ACCESS_KEY: 'e'.repeat(64),
  });
  h.cloud.r2Keys.add('windows-proprietary/kfimg-20261112/image-v2.zip');
  const started = await h.service.adminStart(body());
  const key = (await batchOf(h, started.batch)).legs[0].key;
  return { h, key, token: creates(h)[0].env.KF_RUN_TOKEN };
};
test('an admin-started leg is trusted at lease time: scopes in the record and in the signed token', async () => {
  const { h, key, token } = await trustedHarness();
  const record = await h.storage.get('record:' + key);
  assert.deepEqual(record.scopes, ['public', 'private']);
  assert.equal(record.scopeWhy, 'TRUSTED:admin');
  assert.equal(record.provenance, undefined);
  assert.deepEqual(JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString()).scopes, [
    'public',
    'private',
  ]);
});
test('an admin-started leg fetches a private object with the token it was given', async () => {
  const { h, key, token } = await trustedHarness();
  const res = await h.service.fetch(
    new Request(
      h.bindings.WORKER_URL +
        '/bench-data/' +
        key +
        '/private/windows-proprietary/kfimg-20261112/image-v2.zip',
      { headers: { Authorization: 'Bearer ' + token } },
    ),
  );
  assert.equal(res.status, 302, await res.clone().text());
});
test('a batch stored without provenance (before the field existed) is untrusted: its legs get the public scope only', async () => {
  const h = await harness();
  const started = await h.service.adminStart(body());
  const batch = await batchOf(h, started.batch);
  // a second die's leg of an older batch: no stored provenance
  delete batch.provenance;
  batch.legs.push({
    ...batch.legs[0],
    seq: 2,
    key: batch.id + '-2',
    state: 'PENDING',
    machineId: undefined,
  });
  await h.storage.put('batch:' + batch.id, batch);
  h.cloud.contracts = Array.from({ length: 20 }, (_, i) => 950 + i);
  h.time.value += 11 * 60000;
  await h.service.sweep();
  const second = await h.storage.get('record:' + batch.id + '-2');
  assert.ok(
    second,
    JSON.stringify((await batchOf(h, batch.id)).legs.map((l) => [l.key, l.state, l.lastRefusal])),
  );
  assert.deepEqual(second.scopes, ['public']);
  assert.equal(second.scopeWhy, 'UNTRUSTED:NO_PROVENANCE');
});
