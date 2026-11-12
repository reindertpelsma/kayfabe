import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { RELEASE_MATRIX, vendorOfCpuName } from '../src/release-matrix.js';
import { releaseVerdict, wantsRerun } from '../src/release.js';
import { configuration } from '../src/policy.js';
import { applyProfile } from '../src/tier.js';
import { setup, env, sha } from './helpers.js';
import { matrixTest, capablePayload, receipts, subrunSuites } from './release-fixture.js';

// Release resilience (owner 2026-11-13): "rentals can fail without meaning the test failed". A rental that is refused, an
// offer that disappears, a box that never boots or dies mid-run, and a harness failure replace THAT leg on a fresh box while
// the other boxes keep running; only a guest failure is a FAIL.
// FALSIFIERS (written before the tests; docs/design/V3_RELEASE_RESILIENCE.md section 1): a failed or vanished rental yields
// FAIL; a replacement ignores the die, the vendor pin, the recognised-AMD/Intel rule, ARM exclusion or the slot/cost caps;
// a replacement of lane A leaves lane B waiting forever or on the same machine; a healthy leg is cancelled, restarted or
// rented twice; the re-run allowance is exceeded; the release ends PASS with a vendor, lane or die uncovered; the cost cap
// is bypassed by re-runs.
const AMD = 'AMD EPYC 7B13 64-Core Processor';
const INTEL = 'Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz';
const synthetic = () =>
  JSON.parse(readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)))
    .offers;
// One template row per matrix die (cheapest synthetic row of that die).
const TEMPLATE = {
  TU106: 7000,
  TU116: 7001,
  GA102: 7990,
  GA104: 7004,
  AD102: 7007,
  AD104: 7005,
  GB202: 7991,
  GB205: 7009,
};
export const LARGE = ['TU106', 'GA102', 'AD102', 'GB202'];
export const MIDDLE = ['TU116', 'GA104', 'AD104', 'GB205'];
// A deep market: per die four offers on four machines of four hosts, prices rising 1 cent per offer, vendors alternating
// (AMD first on the even dies, Intel first on the odd ones) so the cheapest choices already cover both vendors.
// ids: 8000 + 10 * die index + k.
export function deepMarket({ depth = 4, vendors } = {}) {
  const base = synthetic();
  const rows = [];
  Object.entries(TEMPLATE).forEach(([die, id], i) => {
    const t = base.find((o) => o.id === id);
    for (let k = 0; k < depth; k++) {
      const price = Math.round((t.dph_total + 0.01 * k) * 10000) / 10000;
      rows.push({
        ...t,
        id: 8000 + 10 * i + k,
        machine_id: 80000 + 10 * i + k,
        host_id: 18000 + 10 * i + k,
        cpu_name: vendors ? vendors(die, k) : (i + k) % 2 === 0 ? AMD : INTEL,
        dph_total: price,
        dph_base: t.dph_base + 0.01 * k,
        price,
      });
    }
  });
  return rows;
}
const vars = {
  MAX_CONCURRENT_RENTALS: '9',
  RELEASE_ENABLED: 'true',
  HARD_RUNTIME_MINUTES: '300',
  BOOT_LIMIT_MINUTES: '75',
  ROLLING_SPEND_USD: '200',
  DAILY_SPEND_USD: '40',
  ACCOUNT_DAILY_USD: '100',
  MIDDLE_APPS_ENABLED: undefined,
  RELEASE_STAGES: '["specviewperf"]',
};
export async function harness({ market = deepMarket(), extra = {} } = {}) {
  const h = await setup({ ...vars, ...extra });
  h.cloud.credit = 500;
  capablePayload(h);
  h.cloud.extraOffers = market;
  h.cloud.contracts = Array.from({ length: 60 }, (_, i) => 900 + i);
  return h;
}
export const cfgFor = (extra = {}) =>
  applyProfile(configuration({ ...env, ...vars, ...extra }), 'release', 'large');
export const batchOf = (h, id) => h.storage.get('batch:' + id);
export const recordOf = (h, leg) => h.storage.get('record:' + leg.key);
const offerOfMachine = (h, machine) => h.cloud.extraOffers.find((o) => o.machine_id === machine);
// Advance by `minutes` in one-minute-or-longer sweeps; `alive` legs post a log line each time so their heartbeat holds.
export async function advance(h, minutes, alive = [], step = 5) {
  for (let m = 0; m < minutes; m += step) {
    h.time.value += step * 60000;
    for (const leg of alive) {
      const r = await recordOf(h, leg);
      if (r && r.ended === null && r.resultAt === undefined) await beat(h, leg);
    }
    await h.service.sweep();
  }
}
export async function beat(h, leg) {
  const key = leg.key;
  h.seqs ??= new Map();
  const seq = (h.seqs.get(key) ?? 0) + 1;
  h.seqs.set(key, seq);
  const response = await h.report({ seq, kind: 'log', text: 'alive ' + seq }, { run: key });
  assert.equal(response.status, 200, await response.text());
}
// The box posts its result. mode: pass | harness (an infrastructure row fails: RETRY) | guest (a real row fails: FAIL on a
// GREEN host reference) | notgreen (a real row fails but the host reference is not green: UNATTRIBUTED).
export async function report(h, leg, mode = 'pass') {
  const record = await recordOf(h, leg);
  const row = leg.releaseMatrix;
  const offer = offerOfMachine(h, record.machineId);
  let suites = [
    ...['cuda_fat', 'windows_gate', 'windows_gate_2']
      .filter((n) => leg.windowsRuns === 2 || n !== 'windows_gate_2')
      .map((name) => ({ name, pass: name === 'cuda_fat' ? 4 : 1, fail: 0, skip: 0 })),
    ...subrunSuites(row),
  ];
  if (leg.spec) suites.push({ name: 'specviewperf_' + leg.spec, pass: 1, fail: 0, skip: 0 });
  if (record.middleAppsRequested)
    suites.push(
      { name: 'apps', pass: 66, fail: 0, skip: 0 },
      { name: 'windows_apps', pass: 10, fail: 0, skip: 0 },
    );
  if (mode === 'harness')
    suites = [
      { name: 'cuda_fat', pass: 4, fail: 0, skip: 0 },
      { name: 'infrastructure', pass: 0, fail: 1, skip: 0 },
    ];
  if (mode === 'guest' || mode === 'notgreen')
    suites = [
      { name: 'cuda_fat', pass: 4, fail: 0, skip: 0 },
      { name: 'windows_gate', pass: 0, fail: 1, skip: 0 },
    ];
  const pass = mode === 'pass';
  const response = await h.report(
    {
      seq: 1000,
      kind: 'result',
      result: {
        nonce: record.nonce,
        verdict: pass ? 'PASS' : 'FAIL',
        host_reference: mode === 'notgreen' ? 'NOT_GREEN' : 'GREEN',
        suites,
        ...(pass ? { matrix_receipts: receipts(row) } : {}),
        host_claims: { cpu_vendor: vendorOfCpuName(offer.cpu_name) },
        hardware: {
          gpu_name: offer.gpu_name,
          compute_cap: offer.compute_cap,
          driver_version: row.drivers.host_driver,
        },
      },
    },
    { run: leg.key },
  );
  assert.equal(response.status, 200, mode + ' ' + leg.key + ' ' + (await response.text()));
}
export async function start(h) {
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  return id;
}
export const settle = async (h) => {
  h.time.value += 11 * 60000;
  await h.service.sweep();
};
export const legOf = (batch, die, spec) =>
  batch.legs.filter((l) => l.die === die && (spec === undefined || l.spec === spec));
export const openRecords = async (h) =>
  [...(await h.storage.list({ prefix: 'record:' })).values()].filter((r) => r.ended === null);
export { RELEASE_MATRIX, releaseVerdict, wantsRerun, matrixTest, receipts, subrunSuites, sha };
export { INTEL, AMD, offerOfMachine };

// Let every open leg pass, sweep, and repeat until the batch has nothing pending or running (lane B legs and replacements
// start on later sweeps).
export async function finishAll(h, id, mode = 'pass', rounds = 6) {
  for (let i = 0; i < rounds; i++) {
    const batch = await batchOf(h, id);
    if (!batch.legs.some((l) => l.state === 'PENDING' || l.state === 'STARTED')) break;
    for (const leg of batch.legs) {
      const r = await recordOf(h, leg);
      if (leg.state === 'STARTED' && r && r.ended === null && r.resultAt === undefined)
        await report(h, leg, mode);
    }
    await settle(h);
  }
  return batchOf(h, id);
}
// The plan's chosen offer for one leg (die + spec) before the release starts.
export async function plannedOffer(h, die, spec) {
  const { plan } = await h.service.adminPlan({ commit_sha: sha, tier: 'large' });
  return plan.families
    .flatMap((f) => f.cells)
    .find((c) => c.die === die)
    .legs.find((l) => l.spec === spec);
}
