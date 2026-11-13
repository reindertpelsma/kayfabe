import test from 'node:test';
import assert from 'node:assert/strict';
import {
  harness,
  start,
  batchOf,
  recordOf,
  report,
  beat,
  advance,
  settle,
  legOf,
  openRecords,
  offerOfMachine,
  deepMarket,
  cfgFor,
  matrixTest,
  RELEASE_MATRIX,
  releaseVerdict,
  wantsRerun,
  sha,
  LARGE,
  MIDDLE,
  AMD,
  INTEL,
} from './release-resilience-fixture.js';
import { vendorOfCpuName } from '../src/release-matrix.js';
import { vendorCover, movePin, pinForRerun } from '../src/release.js';

import { plannedOffer, finishAll } from './release-resilience-fixture.js';

const MAX_OPEN = 8; // MAX_CONCURRENT_RENTALS 9 minus the master gate's slot
const startedOthers = (batch, leg) =>
  batch.legs.filter((l) => l.key !== leg.key && l.state === 'STARTED');
const offerOf = (h, record) => offerOfMachine(h, record.machineId);
// Slot cap on every observation: never more open boxes than the release may hold.
const checkSlots = async (h) => assert.ok((await openRecords(h)).length <= MAX_OPEN);
// The other legs are exactly as they were: record open, same instance, not destroyed, not re-run, not re-rented.
async function assertUntouched(h, id, healthy) {
  const after = await batchOf(h, id);
  for (const l of healthy) {
    const r = await recordOf(h, l);
    assert.equal(r.ended, null, 'open ' + l.key);
    assert.equal(r.closed, null, 'not closed ' + l.key);
    assert.equal(h.cloud.deletes.includes(String(r.id)), false, 'not destroyed ' + l.key);
    assert.equal(after.legs.filter((x) => x.rerunOf === l.key).length, 0, 'no re-run ' + l.key);
    assert.equal(after.legs.find((x) => x.key === l.key).machineId, l.machineId, 'same machine');
  }
  // no machine was rented twice by two different live legs
  const live = await openRecords(h);
  assert.equal(new Set(live.map((r) => r.machineId)).size, live.length);
}
// The replacement of `failed`: same die, spec and tier, one fresh machine and host, a recognised vendor (the pin's vendor when
// the failed leg carried one), a re-run depth of 1, and a recorded link to the leg it replaces.
async function assertReplacement(h, id, failed, { pinned = failed.cpuVendor } = {}) {
  const batch = await batchOf(h, id);
  const reps = batch.legs.filter((l) => l.rerunOf === failed.key);
  assert.equal(reps.length, 1, 'exactly one replacement');
  const [rep] = reps;
  assert.equal(rep.die, failed.die);
  assert.equal(rep.spec, failed.spec);
  assert.equal(rep.family, failed.family);
  assert.equal(rep.rerunDepth, 1);
  assert.equal(rep.legTier, failed.legTier);
  assert.deepEqual(rep.releaseMatrix, failed.releaseMatrix);
  if (pinned) assert.equal(rep.cpuVendor, pinned);
  const old = await recordOf(h, failed);
  const fresh = await recordOf(h, rep);
  assert.ok(fresh, 'replacement rented');
  assert.notEqual(fresh.machineId, old.machineId);
  assert.notEqual(fresh.hostId, old.hostId);
  assert.equal(fresh.die, failed.die);
  const vendor = vendorOfCpuName(offerOf(h, fresh).cpu_name);
  assert.ok(['amd', 'intel'].includes(vendor));
  assert.equal(fresh.cpuVendorOffered, vendor);
  if (pinned) assert.equal(vendor, pinned);
  assert.equal(fresh.spec, failed.spec);
  assert.equal(fresh.batch, id);
  return { rep, fresh, old };
}
// The release requirements of the matrix, read from the finished batch (independent of releaseVerdict): per die family one
// large die with both SPEC lanes passed on a PASSING leg, one middle die with a passing leg, both vendors confirmed on passing
// large legs, every die of the matrix covered, at least a lane A and lane B on different machines when two machines exist.
function assertMatrixCovered(batch, cfg = cfgFor()) {
  const matrix = RELEASE_MATRIX;
  const pass = (l) => l.state === 'ENDED' && l.outcome?.verdict === 'PASS';
  const dies = matrix.dies;
  assert.equal(dies.length, 8);
  const vendors = { amd: 0, intel: 0 };
  for (const family of Object.keys(cfg.releaseDies)) {
    const mine = dies.filter((d) => d.die_family === family);
    assert.equal(mine.length, cfg.releaseDiesPerFamily, family);
    const large = mine.filter((d) => d.profile !== 'middle'),
      middle = mine.filter((d) => d.profile === 'middle');
    assert.equal(large.length, 1, family + ' has one flagship large die');
    assert.equal(middle.length, 1, family + ' has one middle die');
    const lanes = batch.legs.filter((l) => l.die === large[0].die && pass(l));
    for (const lane of ['lanea', 'laneb'])
      assert.ok(
        lanes.some(
          (l) => l.spec === lane && l.outcome.stageRows['specviewperf_' + lane] === 'PASS',
        ),
        large[0].die + ' ' + lane,
      );
    for (const l of lanes) {
      assert.equal(l.outcome.linux, 'PASS');
      assert.ok(l.outcome.windows.every((w) => w === 'PASS'));
      assert.ok(Object.values(l.outcome.matrix).every((v) => v === 'PASS'));
      if (l.cpuVendorOffered === l.cpuVendorReported && l.cpuVendorOffered in vendors)
        vendors[l.cpuVendorOffered]++;
    }
    const mids = batch.legs.filter((l) => l.die === middle[0].die && pass(l));
    assert.ok(mids.length >= 1, middle[0].die + ' middle leg passed');
    assert.ok(mids.every((l) => l.legTier === 'middle'));
  }
  for (const v of ['amd', 'intel'])
    assert.ok(vendors[v] >= matrix.policy.cpu_vendor_min[v], 'vendor ' + v);
  return vendors;
}

matrixTest(
  '(a) a refused create replaces that leg alone: same die, fresh machine, others untouched',
  async () => {
    const h = await harness();
    const planned = await plannedOffer(h, 'AD102', 'lanea');
    h.cloud.refuseAsks.add(planned.offer_id);
    const id = await start(h);
    let batch = await batchOf(h, id);
    const a = legOf(batch, 'AD102', 'lanea')[0];
    const ra = await recordOf(h, a);
    assert.equal(ra.verdict, 'PROVIDER_FAULT');
    assert.notEqual(ra.ended, null);
    assert.equal(h.cloud.creates, 7);
    const healthy = startedOthers(batch, a);
    await advance(h, 2, healthy, 1);
    await checkSlots(h);
    batch = await batchOf(h, id);
    const failed = batch.legs.find((l) => l.key === a.key);
    assert.equal(failed.outcome.verdict, 'RETRY');
    assert.notEqual(batch.verdict, 'FAIL');
    await assertReplacement(h, id, failed);
    await assertUntouched(
      h,
      id,
      healthy.filter((l) => l.key !== a.key),
    );
    assert.equal(h.cloud.creates, 8);
    assert.deepEqual(h.cloud.deletes, []);
    // the refused offer is never asked for again
    assert.equal(h.cloud.createAttempts.filter((x) => x === planned.offer_id).length, 1);
    const done = await finishAll(h, id);
    assert.equal(done.verdict, 'PASS');
    assert.equal(h.cloud.creates, 12); // 13 asks, one refused
    assert.equal(h.cloud.createAttempts.length, 13);
    assertMatrixCovered(done);
  },
);
matrixTest(
  '(a) a refused create of a vendor-pinned leg is replaced by an offer of the pinned vendor',
  async () => {
    const h = await harness();
    const first = await batchOf(h, await start(h));
    // find a pinned leg of the plan, refuse its offer in a fresh harness
    const pinnedLeg = first.legs.find((l) => l.cpuVendor);
    const h2 = await harness();
    const planned = await plannedOffer(h2, pinnedLeg.die, pinnedLeg.spec);
    h2.cloud.refuseAsks.add(planned.offer_id);
    const id = await start(h2);
    let batch = await batchOf(h2, id);
    const a = batch.legs.find((l) => l.die === pinnedLeg.die && l.spec === pinnedLeg.spec);
    assert.ok(a.cpuVendor);
    await advance(h2, 2, startedOthers(batch, a), 1);
    batch = await batchOf(h2, id);
    const { fresh } = await assertReplacement(
      h2,
      id,
      batch.legs.find((l) => l.key === a.key),
    );
    assert.equal(fresh.cpuVendorOffered, a.cpuVendor);
  },
);
for (const [die, lane] of [
  ['AD102', 'lanea'],
  ['GB202', 'lanea'],
])
  matrixTest(
    '(a) an HTTP 500 on create (' +
      die +
      ') leaves an unproven box: others rent on, the leg is replaced once its boot window has passed',
    async () => {
      const h = await harness();
      const planned = await plannedOffer(h, die, lane);
      h.cloud.errorAsks.add(planned.offer_id);
      const id = await start(h);
      let batch = await batchOf(h, id);
      const a = legOf(batch, die, lane)[0];
      // the unproven create did not stop the legs after it (it holds a recorded rental, so they go on in the same tick)
      assert.equal(h.cloud.creates, 7);
      assert.equal(a.state, 'STARTED');
      const healthy = startedOthers(batch, a);
      await advance(h, 10, healthy, 1);
      batch = await batchOf(h, id);
      assert.equal(
        batch.legs.filter((l) => l.rerunOf).length,
        0,
        'not replaced before the boot window ends',
      );
      assert.notEqual(batch.verdict, 'FAIL');
      // Vast never showed an instance: after the boot limit the Worker closes it as absent and replaces the leg
      await advance(h, 80, healthy, 5);
      batch = await batchOf(h, id);
      assert.notEqual(batch.verdict, 'FAIL');
      const failed = batch.legs.find((l) => l.key === a.key);
      assert.equal(failed.outcome.verdict, 'RETRY');
      await assertReplacement(h, id, failed);
      await checkSlots(h);
      await assertUntouched(h, id, healthy);
    },
  );

matrixTest(
  '(b) offers that vanish between plan and rent are replaced at rent time, nothing fails',
  async () => {
    const h = await harness();
    const id = await start(h);
    const batch = await batchOf(h, id);
    // the two cheapest offers of every die disappear before lane B rents
    for (let i = 0; i < 8; i++) for (const k of [0, 1]) h.cloud.hiddenOffers.add(8000 + 10 * i + k);
    const done = await finishAll(h, id);
    assert.equal(done.verdict, 'PASS');
    assert.equal(done.legs.filter((l) => l.rerunOf).length, 0);
    assert.equal(h.cloud.creates, 12);
    for (const l of done.legs.filter((x) => x.spec === 'laneb')) {
      const r = await recordOf(h, l);
      assert.ok(r.machineId >= 80002 && r.machineId % 10 >= 2, 'lane B took a surviving offer');
      const a = done.legs.find((x) => x.die === l.die && x.spec === 'lanea');
      assert.notEqual(r.machineId, a.machineId);
    }
    assertMatrixCovered(done);
  },
);
matrixTest(
  '(b) the vendor pin of a leg whose vendor vanished moves to another leg and the release still covers both vendors',
  async () => {
    const h = await harness();
    const id = await start(h);
    const batch = await batchOf(h, id);
    const pinned = batch.legs.find((l) => l.cpuVendor === 'intel' && l.state === 'PENDING');
    assert.ok(pinned, 'an intel-pinned leg still waits to rent');
    const index = Object.keys({
      TU106: 0,
      TU116: 0,
      GA102: 0,
      GA104: 0,
      AD102: 0,
      AD104: 0,
      GB202: 0,
      GB205: 0,
    }).indexOf(pinned.die);
    // every intel host of that die is gone
    for (let k = 0; k < 4; k++)
      if (
        vendorOfCpuName(
          h.cloud.extraOffers.find((o) => o.id === 8000 + 10 * index + k).cpu_name,
        ) === 'intel'
      )
        h.cloud.hiddenOffers.add(8000 + 10 * index + k);
    const done = await finishAll(h, id);
    assert.equal(
      done.verdict,
      'PASS',
      JSON.stringify(done.cells?.filter((c) => c.verdict !== 'PASS')),
    );
    assert.equal(done.legs.filter((l) => l.cpuVendor === 'intel').length, 1, 'one intel pin');
    assert.equal(done.legs.find((l) => l.key === pinned.key).cpuVendor, undefined);
    assertMatrixCovered(done);
  },
);
matrixTest(
  '(b) a die with no offer left leaves its leg MISSING and the release INCOMPLETE, never FAIL',
  async () => {
    const h = await harness();
    const id = await start(h);
    const index = 2; // GA102
    for (let k = 0; k < 4; k++) h.cloud.hiddenOffers.add(8000 + 10 * index + k);
    let batch = await finishAll(h, id, 'pass', 3);
    h.time.value += 400 * 60000;
    await h.service.sweep();
    batch = await batchOf(h, id);
    const b = legOf(batch, 'GA102', 'laneb')[0];
    assert.equal(b.state, 'MISSING');
    assert.equal(b.lastRefusal, 'NO_ALLOWED_OFFERS');
    assert.equal(batch.verdict, 'INCOMPLETE');
    assert.equal(batch.state, 'DONE');
    assert.equal(h.cloud.creates, 11);
  },
);

matrixTest(
  '(c) a box that never reports is closed at the boot limit and its leg replaced while the others run on',
  async () => {
    const h = await harness();
    const id = await start(h);
    let batch = await batchOf(h, id);
    const dead = legOf(batch, 'GB202', 'lanea')[0];
    const healthy = startedOthers(batch, dead);
    for (const l of healthy) await beat(h, l);
    await advance(h, 70, healthy, 5);
    assert.equal((await recordOf(h, dead)).ended, null, 'still inside the boot window');
    await advance(h, 10, healthy, 1);
    await checkSlots(h);
    batch = await batchOf(h, id);
    const failed = batch.legs.find((l) => l.key === dead.key);
    assert.equal(failed.outcome.verdict, 'RETRY');
    assert.equal((await recordOf(h, dead)).verdict, 'PROVIDER_FAULT');
    assert.notEqual(batch.verdict, 'FAIL');
    await assertReplacement(h, id, failed);
    await assertUntouched(h, id, healthy);
    assert.equal(h.cloud.deletes.length, 1, 'only the dead box was destroyed');
  },
);

for (const how of ['exited', 'absent'])
  matrixTest(
    '(d) a box that dies mid-run (' + how + ') is replaced; lane B never shares its machine',
    async () => {
      const h = await harness();
      const id = await start(h);
      let batch = await batchOf(h, id);
      const a = legOf(batch, 'AD102', 'lanea')[0];
      const healthy = startedOthers(batch, a);
      for (const l of [a, ...healthy]) await beat(h, l);
      await advance(h, 20, [a, ...healthy], 5);
      const ra = await recordOf(h, a);
      if (how === 'exited') h.cloud.instances.get(String(ra.id)).actual_status = 'exited';
      else h.cloud.instances.delete(String(ra.id));
      await advance(h, 30, healthy, 2);
      await checkSlots(h);
      batch = await batchOf(h, id);
      const failed = batch.legs.find((l) => l.key === a.key);
      assert.equal(failed.outcome.verdict, 'RETRY');
      assert.notEqual(batch.verdict, 'FAIL');
      const { fresh } = await assertReplacement(h, id, failed);
      await assertUntouched(h, id, healthy);
      // eight boxes are open again (the replacement took the freed slot), so lane B waits for a free slot
      assert.equal(
        await recordOf(
          h,
          batch.legs.find((l) => l.die === 'AD102' && l.spec === 'laneb'),
        ),
        undefined,
      );
      const done = await finishAll(h, id);
      assert.equal(done.verdict, 'PASS');
      // lane B started only after lane A's original settled, on neither the dead nor the replacement machine
      const b = done.legs.find((l) => l.die === 'AD102' && l.spec === 'laneb');
      const rb = await recordOf(h, b);
      assert.ok(rb.started >= (await recordOf(h, a)).ended);
      assert.notEqual(rb.machineId, ra.machineId);
      assert.notEqual(rb.machineId, fresh.machineId);
      assertMatrixCovered(done);
    },
  );

matrixTest(
  '(e) a harness failure or a not-green host is re-run; a guest FAIL stays FAIL and is never re-run',
  async () => {
    const h = await harness();
    const id = await start(h);
    let batch = await batchOf(h, id);
    const harnessLeg = legOf(batch, 'TU116')[0],
      notGreen = legOf(batch, 'GA104')[0],
      guest = legOf(batch, 'AD104')[0];
    const rest = batch.legs.filter(
      (l) => l.state === 'STARTED' && ![harnessLeg, notGreen, guest].some((x) => x.key === l.key),
    );
    await report(h, harnessLeg, 'harness');
    await report(h, notGreen, 'notgreen');
    await report(h, guest, 'guest');
    for (const l of rest) await beat(h, l);
    await advance(h, 12, rest, 1);
    batch = await batchOf(h, id);
    assert.equal(batch.legs.find((l) => l.key === harnessLeg.key).outcome.verdict, 'RETRY');
    assert.equal(batch.legs.find((l) => l.key === notGreen.key).outcome.verdict, 'UNATTRIBUTED');
    assert.equal(batch.legs.find((l) => l.key === guest.key).outcome.verdict, 'FAIL');
    await assertReplacement(h, id, harnessLeg);
    await assertReplacement(h, id, notGreen);
    assert.equal(batch.legs.filter((l) => l.rerunOf === guest.key).length, 0);
    assert.equal(batch.verdict, 'FAIL');
    await assertUntouched(h, id, rest);
    const done = await finishAll(h, id);
    assert.equal(done.verdict, 'FAIL', 'the guest FAIL decides the release');
  },
);

matrixTest(
  '(f) an owner stop replaces nothing, rents nothing and is INCOMPLETE, never PASS or FAIL',
  async () => {
    const h = await harness();
    const id = await start(h);
    const batch = await batchOf(h, id);
    const failing = legOf(batch, 'TU116')[0];
    await h.service.adminStopBatch(id);
    await advance(h, 3, [], 1);
    const after = await batchOf(h, id);
    assert.equal(after.state, 'STOPPED');
    assert.equal(after.legs.filter((l) => l.rerunOf).length, 0);
    assert.equal(h.cloud.creates, 8);
    assert.ok(
      after.legs.filter((l) => l.state === 'MISSING').length === 4,
      'the 4 waiting lane B legs',
    );
    assert.ok(
      after.legs
        .filter((l) => l.state === 'ENDED')
        .every((l) => l.outcome.verdict === 'UNATTRIBUTED'),
    );
    assert.equal(after.verdict, 'INCOMPLETE');
    assert.equal((await openRecords(h)).length, 0);
    assert.equal(
      wantsRerun(
        after.legs.find((l) => l.key === failing.key),
        5,
      ),
      false,
    );
  },
);

for (const reruns of [1, 2])
  matrixTest(
    'a leg is re-run at most RELEASE_RERUNS_PER_LEG=' +
      reruns +
      ' times, then the release is INCOMPLETE',
    async () => {
      const h = await harness({
        market: deepMarket({ depth: 8 }),
        extra: { RELEASE_RERUNS_PER_LEG: String(reruns) },
      });
      for (let k = 0; k < 8; k++) h.cloud.refuseAsks.add(8040 + k); // every AD102 offer is refused
      const id = await start(h);
      const done = await finishAll(h, id, 'pass', 10);
      h.time.value += 400 * 60000;
      await h.service.sweep();
      const batch = await batchOf(h, id);
      for (const lane of ['lanea', 'laneb']) {
        const lineage = batch.legs.filter((l) => l.die === 'AD102' && l.spec === lane);
        assert.equal(lineage.length, 1 + reruns, lane);
        assert.equal(Math.max(...lineage.map((l) => l.rerunDepth)), reruns);
      }
      assert.ok(batch.legs.every((l) => l.rerunDepth <= reruns));
      assert.equal(
        h.cloud.createAttempts.filter((x) => x >= 8040 && x < 8048).length,
        2 * (1 + reruns),
      );
      assert.notEqual(batch.verdict, 'FAIL');
      assert.equal(batch.verdict, 'INCOMPLETE');
      assert.equal(batch.state, 'DONE');
      assert.deepEqual(
        batch.cells
          .filter((c) => c.verdict !== 'PASS')
          .map((c) => c.die)
          .filter((d, i, a) => a.indexOf(d) === i),
        ['AD102'],
      );
      assert.equal(done.legs.length > 0, true);
    },
  );

for (const room of [0, 2])
  matrixTest(
    're-runs never pass the release cost cap (' +
      room +
      ' USD above the plan): a dearer replacement that does not fit waits for budget',
    async () => {
      // the two cheapest offers of a die are the plan's; the rest are dear (1.6 USD/h, under the 2 USD/h price cap). The cheap
      // second offers vanish after the start, so the replacements cost far more than the plan.
      const vendors = (die, k) =>
        ((['TU106', 'AD102'].includes(die) ? 0 : 1) + k) % 2 === 0 ? AMD : INTEL;
      const dear = deepMarket({ vendors }).map((o) =>
        o.id % 10 < 2
          ? o
          : { ...o, price: 1.6, dph_total: 1.6, dph_base: o.dph_base + (1.6 - o.dph_total) },
      );
      const probe = await harness({ market: dear });
      const { plan } = await probe.service.adminPlan({ commit_sha: sha, tier: 'large' });
      assert.equal(plan.admitted, true, plan.refusals.join());
      const cap = Math.ceil(plan.estimate_usd * 100) / 100 + room;
      const h = await harness({ market: dear, extra: { RELEASE_COST_CAP_USD: String(cap) } });
      const id = await start(h);
      let batch = await batchOf(h, id);
      for (let i = 0; i < 8; i++) h.cloud.hiddenOffers.add(8001 + 10 * i);
      const lanesA = batch.legs.filter((l) => l.spec === 'lanea');
      const healthy = batch.legs.filter((l) => l.state === 'STARTED' && l.spec !== 'lanea');
      for (const l of batch.legs.filter((x) => x.state === 'STARTED')) await beat(h, l);
      // the boxes run long, so their settled cost is close to the worst case the cap was planned on
      await advance(
        h,
        190,
        batch.legs.filter((x) => x.state === 'STARTED'),
        10,
      );
      for (const l of lanesA)
        h.cloud.instances.get(String((await recordOf(h, l)).id)).actual_status = 'exited';
      const spent = [];
      let waited = 0;
      for (let i = 0; i < 15; i++) {
        await advance(h, 2, healthy, 2);
        batch = await batchOf(h, id);
        spent.push(batch.spentUsd);
        // a replacement that does not fit waits (PENDING, refused by the cap) and the rental is not made
        waited += batch.legs.filter(
          (l) => l.rerunOf && l.state === 'PENDING' && /release-cost-cap/.test(l.lastRefusal ?? ''),
        ).length;
        await checkSlots(h);
      }
      assert.ok(waited >= 1, 'a replacement was refused by the cap and waited');
      assert.ok(
        batch.legs.filter((l) => l.rerunOf && l.state === 'STARTED').length >= 1,
        'a replacement fit',
      );
      for (let i = 0; i < 8; i++) {
        batch = await batchOf(h, id);
        for (const l of batch.legs) {
          const r = await recordOf(h, l);
          if (l.state === 'STARTED' && r && r.ended === null && r.resultAt === undefined)
            await report(h, l);
        }
        await settle(h);
        spent.push((await batchOf(h, id)).spentUsd);
        await checkSlots(h);
      }
      h.time.value += 400 * 60000;
      await h.service.sweep();
      batch = await batchOf(h, id);
      spent.push(batch.spentUsd);
      assert.ok(
        spent.every((x) => x <= cap + 1e-9),
        'liability never above the cap ' + cap + ': ' + spent.join(),
      );
      // the cap held at every observation, and whatever was left out is INCOMPLETE, never FAIL
      assert.notEqual(batch.verdict, 'FAIL');
      assert.ok(['PASS', 'INCOMPLETE'].includes(batch.verdict));
      assert.ok(batch.legs.every((l) => l.rerunDepth <= 1));
    },
  );
matrixTest('with a cost cap that leaves room the replacement is rented', async () => {
  const h = await harness({ extra: { RELEASE_COST_CAP_USD: '40' } });
  const id = await start(h);
  let batch = await batchOf(h, id);
  const a = legOf(batch, 'AD102', 'lanea')[0];
  const healthy = startedOthers(batch, a);
  for (const l of [a, ...healthy]) await beat(h, l);
  await advance(h, 20, [a, ...healthy], 5);
  h.cloud.instances.get(String((await recordOf(h, a)).id)).actual_status = 'exited';
  await advance(h, 20, healthy, 2);
  batch = await batchOf(h, id);
  assert.equal(batch.legs.filter((l) => l.rerunOf === a.key && l.state === 'STARTED').length, 1);
  assert.ok(batch.spentUsd <= 40);
});

matrixTest(
  'both vendors are required: when the only second vendor cannot be rented the release is INCOMPLETE with CPU_VENDOR_MISSING',
  async () => {
    // all AMD, except the two Intel offers of TU106
    const market = deepMarket({
      vendors: (die, k) => (die === 'TU106' && k % 2 === 1 ? INTEL : AMD),
    });
    const h = await harness({ market });
    h.cloud.refuseAsks.add(8001);
    h.cloud.refuseAsks.add(8003);
    const id = await start(h);
    const done = await finishAll(h, id, 'pass', 10);
    assert.notEqual(done.verdict, 'FAIL');
    assert.equal(done.verdict, 'INCOMPLETE');
    const view = releaseVerdict(done);
    assert.equal(view.reason, 'CPU_VENDOR_MISSING');
    assert.deepEqual(view.cpu_vendor.missing, ['intel']);
    assert.ok(done.legs.every((l) => l.rerunDepth <= 1));
    // only the die that could not rent its Intel box is incomplete; every other cell passed
    assert.ok(done.cells.filter((c) => c.die !== 'TU106').every((c) => c.verdict === 'PASS'));
    assert.ok(done.cells.filter((c) => c.die === 'TU106').some((c) => c.verdict !== 'PASS'));
  },
);

for (const how of ['exited', 'absent'])
  matrixTest(
    'a die with one machine (' +
      how +
      '): a lane A that died is not replaced on its machine and lane B does not rent it again',
    async () => {
      const market = deepMarket().filter((o) => !(o.id >= 8041 && o.id <= 8043)); // AD102: only 8040 is left
      const h = await harness({ market });
      const id = await start(h);
      let batch = await batchOf(h, id);
      const a = legOf(batch, 'AD102', 'lanea')[0];
      assert.equal(legOf(batch, 'AD102', 'laneb')[0].reuseMachine, true);
      const healthy = startedOthers(batch, a);
      for (const l of [a, ...healthy]) await beat(h, l);
      await advance(h, 10, [a, ...healthy], 5);
      if (how === 'exited')
        h.cloud.instances.get(String((await recordOf(h, a)).id)).actual_status = 'exited';
      else h.cloud.instances.delete(String((await recordOf(h, a)).id));
      await advance(h, 30, healthy, 2);
      for (let i = 0; i < 4; i++) {
        batch = await batchOf(h, id);
        for (const l of batch.legs) {
          const r = await recordOf(h, l);
          if (l.state === 'STARTED' && r && r.ended === null && r.resultAt === undefined)
            await report(h, l);
        }
        await settle(h);
      }
      h.time.value += 400 * 60000;
      await h.service.sweep();
      batch = await batchOf(h, id);
      assert.equal(
        h.cloud.createAttempts.filter((x) => x === 8040).length,
        1,
        'the machine that died is not rented again',
      );
      assert.ok(batch.legs.filter((l) => l.die === 'AD102' && l.state === 'MISSING').length >= 1);
      assert.equal(batch.verdict, 'INCOMPLETE');
    },
  );

matrixTest(
  'all failure kinds at once: every leg is replaced alone, slots and re-run limits hold, the matrix is covered before PASS',
  async () => {
    const h = await harness();
    const plannedA = await plannedOffer(h, 'AD102', 'lanea');
    h.cloud.refuseAsks.add(plannedA.offer_id); // (a) refused create
    const id = await start(h);
    let batch = await batchOf(h, id);
    const refused = legOf(batch, 'AD102', 'lanea')[0];
    const died = legOf(batch, 'TU116')[0], // (d) dies mid-run
      harnessFault = legOf(batch, 'GB205')[0], // (e) harness failure
      notGreen = legOf(batch, 'GA104')[0]; // (e) not-green host
    const rest = batch.legs.filter(
      (l) =>
        l.state === 'STARTED' &&
        ![refused, died, harnessFault, notGreen].some((x) => x.key === l.key),
    );
    const alive = [died, harnessFault, notGreen, ...rest];
    for (const l of alive) await beat(h, l);
    await advance(h, 5, alive, 5);
    h.cloud.instances.get(String((await recordOf(h, died)).id)).actual_status = 'exited';
    await report(h, harnessFault, 'harness');
    await report(h, notGreen, 'notgreen');
    let peak = 0;
    for (let i = 0; i < 40; i++) {
      await advance(h, 2, rest, 2);
      const open = (await openRecords(h)).length;
      peak = Math.max(peak, open);
      assert.ok(open <= MAX_OPEN);
      if ((await batchOf(h, id)).legs.filter((l) => l.rerunOf).length === 4) break;
    }
    batch = await batchOf(h, id);
    for (const f of [refused, died, harnessFault, notGreen]) {
      const failed = batch.legs.find((l) => l.key === f.key);
      assert.ok(['RETRY', 'UNATTRIBUTED'].includes(failed.outcome.verdict), f.key);
      await assertReplacement(h, id, failed);
    }
    assert.notEqual(batch.verdict, 'FAIL');
    await assertUntouched(h, id, rest);
    const done = await finishAll(h, id, 'pass', 10);
    assert.equal(
      done.verdict,
      'PASS',
      JSON.stringify(done.cells.filter((c) => c.verdict !== 'PASS')),
    );
    assert.equal(done.legs.filter((l) => l.rerunOf).length, 4);
    assert.ok(done.legs.every((l) => l.rerunDepth <= 1));
    assert.equal(h.cloud.creates, 12 + 4 - 1);
    assert.ok(done.spentUsd <= done.costCapUsd);
    assert.equal(peak <= MAX_OPEN, true);
    const vendors = assertMatrixCovered(done);
    assert.ok(vendors.amd >= 1 && vendors.intel >= 1);
    // the independent checker is not vacuous: remove one requirement at a time and it must object, and so must the judge
    const damage = {
      'a SPEC lane': (b) =>
        (b.legs.find(
          (l) => l.die === 'GB202' && l.spec === 'laneb' && l.outcome.verdict === 'PASS',
        ).outcome.verdict = 'RETRY'),
      'a middle die': (b) =>
        b.legs
          .filter((l) => l.legTier === 'middle' && l.die === 'AD104')
          .forEach((l) => (l.outcome.verdict = 'RETRY')),
      'the intel cover': (b) =>
        b.legs
          .filter((l) => l.cpuVendorOffered === 'intel')
          .forEach((l) => (l.cpuVendorReported = 'amd')),
    };
    for (const [what, hurt] of Object.entries(damage)) {
      const copy = structuredClone(done);
      hurt(copy);
      assert.throws(() => assertMatrixCovered(copy), what);
      if (what !== 'a SPEC lane' && what !== 'a middle die')
        assert.notEqual(releaseVerdict(copy).verdict, 'PASS', what);
    }
  },
);

matrixTest(
  'the checker and the judge agree: a finished release with an uncovered lane, die or vendor is never PASS',
  async () => {
    const h = await harness();
    const id = await start(h);
    const done = await finishAll(h, id);
    assert.equal(done.verdict, 'PASS');
    assertMatrixCovered(done);
    const lane = structuredClone(done);
    const l = lane.legs.find((x) => x.die === 'GB202' && x.spec === 'laneb');
    l.outcome = {
      ...l.outcome,
      verdict: 'RETRY',
      stageRows: { ...l.outcome.stageRows, specviewperf_laneb: 'RETRY' },
      stages: { specviewperf: 'RETRY' },
    };
    assert.equal(releaseVerdict(lane).verdict, 'INCOMPLETE');
    assert.throws(() => assertMatrixCovered(lane));
    const mid = structuredClone(done);
    mid.legs = mid.legs.filter((x) => x.die !== 'GA104');
    assert.notEqual(releaseVerdict(mid).verdict, 'PASS');
    assert.throws(() => assertMatrixCovered(mid));
  },
);

matrixTest(
  'a vendor that only unpinned legs still cover is kept by the replacement of the last of them',
  async () => {
    // TU106: AMD first, Intel second; every other die: Intel first, AMD second. The AMD pin is TU106 lane A, the Intel pin TU106 lane B.
    const vendors = (die, k) =>
      die === 'TU106' ? (k % 2 === 0 ? AMD : INTEL) : k % 2 === 0 ? INTEL : AMD;
    const h = await harness({ market: deepMarket({ vendors }) });
    const id = await start(h);
    let batch = await batchOf(h, id);
    const pinned = batch.legs.find((l) => l.cpuVendor === 'intel');
    assert.equal(pinned.die, 'TU106');
    const cover = batch.legs.filter((l) => l.spec === 'lanea' && l.die !== 'TU106');
    assert.equal(cover.length, 3);
    // every Intel host of TU106 is gone: the pin is dropped because three running Intel legs already cover the vendor
    for (const k of [1, 3]) h.cloud.hiddenOffers.add(8000 + k);
    const tu106a = legOf(batch, 'TU106', 'lanea')[0];
    const healthy = batch.legs.filter(
      (l) => l.state === 'STARTED' && !cover.includes(l) && l.key !== tu106a.key,
    );
    const alive = [...cover, ...healthy];
    for (const l of alive) await beat(h, l);
    await report(h, tu106a); // lane A settles, so lane B tries to rent and finds no Intel host
    await advance(h, 12, alive, 1);
    batch = await batchOf(h, id);
    assert.equal(
      batch.legs.filter((l) => l.cpuVendor === 'intel').length,
      0,
      'no leg is pinned any more',
    );
    // all three running Intel legs die: the replacement of the last one is pinned to Intel, or the vendor would be lost
    for (const l of cover)
      h.cloud.instances.get(String((await recordOf(h, l)).id)).actual_status = 'exited';
    await advance(h, 30, healthy, 2);
    batch = await batchOf(h, id);
    const reps = cover.map((l) => batch.legs.find((x) => x.rerunOf === l.key));
    assert.ok(reps.every(Boolean));
    assert.equal(reps.filter((l) => l.cpuVendor === 'intel').length, 1);
    const done = await finishAll(h, id, 'pass', 12);
    assert.equal(
      done.verdict,
      'PASS',
      JSON.stringify(done.cells?.filter((c) => c.verdict !== 'PASS')),
    );
    assertMatrixCovered(done);
  },
);

// The vendor-cover helpers on their own (pure functions over a batch).
const min = { amd: 1, intel: 1 };
const leg = (extra) => ({ legTier: 'large', state: 'PENDING', ...extra });
const unitBatch = () => ({
  legs: [
    leg({ key: 'a', cpuVendor: 'intel' }),
    leg({ key: 'b' }),
    leg({ key: 'c', legTier: 'middle' }),
    leg({ key: 'd', state: 'STARTED', cpuVendorOffered: 'amd' }),
    leg({ key: 'e', state: 'ENDED', cpuVendorOffered: 'intel', outcome: { verdict: 'RETRY' } }),
  ],
});
test('vendorCover counts waiting pins and running or passed legs of the vendor, never an ended RETRY leg', () => {
  const b = unitBatch();
  assert.deepEqual(
    vendorCover(b, 'intel').map((l) => l.key),
    ['a'],
  );
  assert.deepEqual(
    vendorCover(b, 'amd').map((l) => l.key),
    ['d'],
  );
  assert.deepEqual(
    vendorCover(b, 'intel', b.legs[0]).map((l) => l.key),
    [],
  );
  b.legs[4].outcome.verdict = 'PASS';
  assert.deepEqual(
    vendorCover(b, 'intel').map((l) => l.key),
    ['a', 'e'],
  );
});
test('movePin gives the pin to the next waiting large leg and the leg rents unpinned', () => {
  const b = unitBatch();
  assert.equal(movePin(b, b.legs[0], min).key, 'b');
  assert.equal(b.legs[0].cpuVendor, undefined);
  assert.equal(b.legs[1].cpuVendor, 'intel');
  assert.equal(movePin(b, b.legs[1], min), null, 'no other waiting large leg is left');
  assert.equal(b.legs[1].cpuVendor, undefined);
  assert.deepEqual(b.legs[1].pinFailed, ['intel']);
});
test('movePin never gives a pin to a middle leg or to a leg that already failed to find the vendor', () => {
  const b = unitBatch();
  b.legs[1].state = 'STARTED';
  assert.equal(movePin(b, b.legs[0], min), null, 'the only other waiting leg is a middle leg');
  assert.equal(b.legs[2].cpuVendor, undefined);
  const c = unitBatch();
  c.legs[1].pinFailed = ['intel'];
  assert.equal(movePin(c, c.legs[0], min), null);
});
test('movePin drops the pin without pinning another leg when the others already cover the vendor', () => {
  const b = unitBatch();
  b.legs[3].cpuVendorOffered = 'intel';
  assert.equal(movePin(b, b.legs[0], min), null);
  assert.equal(b.legs[0].cpuVendor, undefined);
  assert.equal(b.legs[1].cpuVendor, undefined);
});
test('pinForRerun keeps the leg pin, else the vendor it rented when nobody else covers it, never for a middle leg', () => {
  const b = unitBatch();
  assert.equal(pinForRerun(b, { cpuVendor: 'amd' }, min), 'amd');
  assert.equal(
    pinForRerun(b, b.legs[4], min),
    undefined,
    'the pending intel pin already covers intel',
  );
  b.legs[0].cpuVendor = undefined;
  assert.equal(pinForRerun(b, b.legs[4], min), 'intel');
  assert.equal(pinForRerun(b, { ...b.legs[4], legTier: 'middle' }, min), undefined);
  assert.equal(pinForRerun(b, b.legs[4], { amd: 0, intel: 0 }), undefined);
});

// Create answers HTTP 500. `ghost`: the instance really is started on the provider (labelled, with the onstart script) before the
// error; otherwise nothing is created. `asks` is a Set of offer ids, or 'all'. `seen` receives (ask, stored batch) at every ask.
function failingCreate(h, asks, { ghost, seen } = {}) {
  const base = h.cloud.fetch;
  h.cloud.fetch = async (url, init) => {
    const m = String(url).match(/\/api\/v0\/asks\/(\d+)/);
    if (m && seen)
      seen.push([Number(m[1]), [...(await h.storage.list({ prefix: 'batch:' })).values()][0]]);
    if (m && (asks === 'all' || asks.has(Number(m[1])))) {
      if (ghost) await base(url, init);
      return new Response('upstream', { status: 500 });
    }
    return base(url, init);
  };
  h.restart();
}
const workerInstances = (h) =>
  [...h.cloud.instances.values()].filter(
    (x) => x.label?.startsWith('kf-ci-') && x.label !== 'kf-ci-unrecorded',
  );
async function sweepUntilDone(h, id, observe) {
  let batch;
  for (let i = 0; i < 120; i++) {
    h.time.value += 5 * 60000;
    await h.service.sweep();
    await observe?.();
    batch = await batchOf(h, id);
    if (batch.state !== 'RUNNING') break;
  }
  return batch;
}

matrixTest(
  '(a) a create that starts the instance and then answers HTTP 500: reconciled, destroyed at the boot limit and charged, one replacement, PASS, no instance left',
  async () => {
    const h = await harness();
    const planned = await plannedOffer(h, 'AD102', 'lanea');
    const seen = [];
    failingCreate(h, new Set([planned.offer_id]), { ghost: true, seen });
    const id = await start(h);
    let batch = await batchOf(h, id);
    const a = legOf(batch, 'AD102', 'lanea')[0];
    assert.equal(a.state, 'STARTED');
    assert.equal(h.cloud.creates, 8, 'the other 7 legs rented in the same tick');
    // the unproven leg was stored as STARTED before the next leg rented, so that leg's contract check sees its liability
    const ghostAt = seen.findIndex(([ask]) => ask === planned.offer_id);
    assert.ok(ghostAt >= 0 && ghostAt < seen.length - 1, 'legs were rented after the ghost');
    for (const [, stored] of seen.slice(ghostAt + 1))
      assert.equal(stored.legs.find((l) => l.key === a.key).state, 'STARTED');
    const healthy = batch.legs.filter((l) => l.key !== a.key && l.state === 'STARTED');
    await advance(h, 5, healthy, 5);
    assert.equal((await recordOf(h, a)).idSource, 'reconcile', 'found by label');
    await advance(h, 90, healthy, 5);
    const ra = await recordOf(h, a);
    assert.equal(ra.verdict, 'PROVIDER_FAULT');
    assert.ok(h.cloud.deletes.includes(String(ra.id)), 'destroyed');
    batch = await batchOf(h, id);
    assert.ok(batch.legs.find((l) => l.key === a.key).costUsd > 0, 'charged');
    assert.equal(batch.legs.filter((l) => l.rerunOf === a.key).length, 1);
    const done = await finishAll(h, id, 'pass', 8);
    assert.equal(done.verdict, 'PASS');
    assertMatrixCovered(done);
    for (const x of workerInstances(h)) {
      const r = await h.storage.get('record:' + x.label.replace('kf-ci-', ''));
      assert.notEqual(r?.ended ?? null, null, 'no open instance left: ' + x.label);
    }
  },
);
for (const ghost of [true, false])
  matrixTest(
    '(a) every create answers HTTP 500 (' +
      (ghost ? 'instances exist' : 'nothing created') +
      '): bounded attempts, boxes and spend, INCOMPLETE',
    async () => {
      const h = await harness();
      failingCreate(h, 'all', { ghost });
      const id = await start(h);
      let maxOpen = 0,
        maxInstances = 0;
      const batch = await sweepUntilDone(h, id, async () => {
        maxOpen = Math.max(maxOpen, (await openRecords(h)).length);
        maxInstances = Math.max(maxInstances, workerInstances(h).length);
      });
      assert.equal(batch.state, 'DONE');
      assert.equal(batch.verdict, 'INCOMPLETE');
      assert.ok(h.cloud.createAttempts.length <= 24, 'at most 12 legs x 2 attempts');
      assert.ok(batch.legs.every((l) => l.rerunDepth <= 1));
      assert.ok(maxOpen <= MAX_OPEN && maxInstances <= MAX_OPEN);
      assert.ok(batch.spentUsd <= batch.costCapUsd);
      assert.equal(workerInstances(h).length, 0, 'every instance that was created is destroyed');
      assert.equal(h.cloud.creates, ghost ? h.cloud.createAttempts.length : 0);
    },
  );
matrixTest(
  'lane A harness RETRY: neither its re-run nor lane B rents the machine that failed',
  async () => {
    const h = await harness();
    const id = await start(h);
    let batch = await batchOf(h, id);
    const a = legOf(batch, 'AD102', 'lanea')[0];
    const ra = await recordOf(h, a);
    await report(h, a, 'harness');
    const others = batch.legs.filter((l) => l.key !== a.key && l.state === 'STARTED');
    await advance(h, 12, others, 1);
    await report(h, legOf(await batchOf(h, id), 'TU116')[0], 'pass');
    await advance(
      h,
      12,
      others.filter((l) => l.die !== 'TU116'),
      1,
    );
    batch = await batchOf(h, id);
    const rerun = batch.legs.find((l) => l.rerunOf === a.key);
    const b = legOf(batch, 'AD102', 'laneb')[0];
    const [rr, rb] = [await recordOf(h, rerun), await recordOf(h, b)];
    assert.ok(rr && rb, 'both rented');
    assert.notEqual(rr.machineId, ra.machineId);
    assert.notEqual(rb.machineId, ra.machineId);
    assert.notEqual(rb.machineId, rr.machineId);
  },
);
