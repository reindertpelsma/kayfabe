import test from 'node:test';
import assert from 'node:assert/strict';
import { BrokerLedger } from '../src/index.js';
import { BrokerError } from '../src/policy.js';
import { setup, sha, env, MemoryStorage } from './helpers.js';
import { FakeD1, d1Skip } from './d1.js';
import { readFileSync } from 'node:fs';
import { requirementsMet } from '../src/results.js';
// wtier, 2026-11-05: admin starts and the release (large) orchestration through the real service
// with the mock provider. FALSIFIERS (written before the code): the plan rents; a start skips an
// admission check a PR rental has (box protocol, caps, concurrency, sha gap for small/middle,
// own-instance ids); a fork or unknown commit rents; a release starts over its cost cap; a second
// Windows leg of a die lands on the first leg's machine; a RETRY counts as PASS or a FAIL is
// re-rented; a leg starts after an owner stop; a re-run reuses the failed machine.
const DIES = [
  ['TU106', 'RTX_2060', 750],
  ['TU116', 'GTX_1660', 750],
  ['GA106', 'RTX_3060', 860],
  ['GA104', 'RTX_3070', 860],
  ['AD104', 'RTX_4070', 890],
  ['AD102', 'RTX_4090', 890],
  ['GB205', 'RTX_5070', 1200],
  ['GB206', 'RTX_5060', 1200],
];
// Two machines for TU116, GA106, AD104, GB205; one for the other four: 12 legs.
const TWICE = ['TU116', 'GA106', 'AD104', 'GB205'];
function releaseMarket(h, { drop = [], twice = TWICE, price = 0.2, sameHost = [] } = {}) {
  const rows = [];
  let id = 10;
  for (const [die, gpu, cc] of DIES) {
    if (drop.includes(die)) continue;
    for (let i = 0; i < (twice.includes(die) ? 2 : 1); i++) {
      id++;
      rows.push({
        ...h.cloud.offer,
        id,
        gpu_name: gpu,
        compute_cap: cc,
        machine_id: 200000 + id,
        // sameHost: both machines of the die on one host (only machine avoidance separates them).
        host_id: sameHost.includes(die) ? 3000 + id - i : 3000 + id,
        gpu_ram: 12288,
        price,
      });
    }
  }
  const [first, ...rest] = rows;
  const { price: p, ...offer } = first;
  h.cloud.offer = offer;
  h.cloud.price = p;
  h.cloud.extraOffers = rest;
  h.cloud.contracts = Array.from({ length: 60 }, (_, i) => 901 + i);
  h.cloud.credit = 500;
  return rows;
}
// Account cap 9: the release takes at most cap - 1 = 8 boxes (review F5); the kill switch is on.
const release = (extra = {}) =>
  setup({
    MAX_CONCURRENT_RENTALS: '9',
    ROLLING_SPEND_USD: '200',
    RELEASE_ENABLED: 'true',
    ...extra,
  });
const creates = (h) =>
  h.cloud.calls.filter((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url)).map((c) => c.body);
const LINUX = { provision: 1, gates: 9, thin_suite: 30, cuda_host: 4, cuda_fat: 4 };
// The box reports its own GPU (review F3): the planned die's board, as nvidia-smi names it.
const boxHardware = (die) => {
  const [, gpu, cc] = DIES.find(([d]) => d === die);
  return {
    gpu_name: 'NVIDIA GeForce ' + gpu.replace('_', ' '),
    compute_cap: cc,
    driver_version: '580.159.04',
  };
};
async function finish(h, key, verdict, spec, hardware) {
  const suites = Object.entries(spec).map(([name, v]) => ({
    name,
    pass: v > 0 ? v : 0,
    fail: v < 0 ? 1 : 0,
    skip: 0,
  }));
  const r = await h.report(
    {
      seq: 1,
      kind: 'result',
      result: {
        nonce: (await h.storage.get('record:' + key)).nonce,
        verdict,
        suites,
        hardware: hardware ?? boxHardware((await h.storage.get('record:' + key)).die),
        host_reference: 'GREEN',
      },
    },
    { run: key },
  );
  assert.equal(r.status, 200, key + ' ' + (await r.text()));
}
const win = (runs) => (runs === 2 ? { windows_gate: 1, windows_gate_2: 1 } : { windows_gate: 1 });
async function tick(h, minutes = 11) {
  h.time.value += minutes * 60000;
  await h.service.sweep();
}
const batchOf = async (h, id) => h.storage.get('batch:' + id);
test('plan: prints the release it WOULD rent, and rents nothing', async () => {
  const h = await release();
  releaseMarket(h);
  const { plan, renting } = await h.service.adminPlan({ commit_sha: sha, tier: 'large' });
  assert.equal(renting, false);
  assert.equal(plan.admitted, true, JSON.stringify(plan.refusals));
  assert.equal(plan.legs, 12);
  assert.equal(plan.parallel, 8);
  assert.deepEqual(
    plan.families.map((f) => f.cells.map((c) => c.die).join('+')),
    ['TU106+TU116', 'GA106+GA104', 'AD104+AD102', 'GB205+GB206'],
  );
  assert.equal(h.cloud.creates, 0);
  assert.equal([...h.storage.data.keys()].filter((k) => /^(record|cost|batch):/.test(k)).length, 0);
});
test('start: a release fans out up to the cap; each leg is a normal rental', async () => {
  const h = await release();
  releaseMarket(h);
  const started = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  assert.equal(started.tier, 'large');
  assert.equal(started.legs.length, 12);
  assert.equal(started.legs.filter((l) => l.state === 'STARTED').length, 8);
  assert.equal(h.cloud.creates, 8);
  const bodies = creates(h);
  for (const body of bodies) {
    assert.equal(body.env.KF_TIER, 'large');
    assert.equal(body.env.KF_MERGE_WINDOWS, '1');
    assert.match(body.label, /^kf-ci-8[0-9]{18}-[0-9]+$/);
    assert.equal(body.env.KF_RUN_ID, body.label.slice('kf-ci-'.length));
    assert.equal(body.env.KF_SHA, sha);
    assert.equal(body.env.KF_STAGES, undefined);
  }
  const batch = await batchOf(h, started.batch);
  // Same-machine dies run Windows twice on their one box; two-machine dies use two machines.
  for (const l of batch.legs.filter((x) => x.state === 'STARTED')) {
    const r = await h.storage.get('record:' + l.key);
    assert.equal(r.die, l.die);
    assert.equal(r.tier, 'large');
    // Review F4: stored as profile check (an older Worker can read and prune it) + budgetClass.
    assert.deepEqual([r.profile, r.budgetClass], ['check', 'release']);
    const spend = await h.storage.get('cost:' + l.key);
    assert.deepEqual([spend.profile, spend.budgetClass], ['check', 'release']);
    assert.equal(r.source, 'admin');
    assert.equal(r.batch, batch.id);
    const body = bodies.find((b) => b.label === r.label);
    assert.equal(body.env.KF_WINDOWS_RUNS, l.windowsRuns === 2 ? '2' : undefined);
    assert.equal(r.hardDeadline - r.started, 300 * 60000);
  }
  for (const die of TWICE) {
    const machines = batch.legs.filter((l) => l.die === die && l.machineId).map((l) => l.machineId);
    assert.equal(new Set(machines).size, machines.length, die);
  }
  // Review F5/F6: the release leaves one slot free and writes no commit-gap marker, so the master
  // gate of the same commit still rents while the release runs.
  assert.equal(await h.storage.get('sha:' + sha), undefined);
  const pr = await h.call();
  assert.equal(pr.status, 200, await pr.clone().text());
  // Only one running release per commit and tier.
  await assert.rejects(
    h.service.adminStart({ commit_sha: sha, tier: 'large' }),
    (e) => e.reason === 'BATCH_RUNNING',
  );
});
test('release: all legs pass on fresh boxes -> PASS; the pending legs start as slots free', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  for (let round = 0; round < 3; round++) {
    const batch = await batchOf(h, id);
    for (const l of batch.legs.filter((x) => x.state === 'STARTED')) {
      const r = await h.storage.get('record:' + l.key);
      if (r.resultAt === undefined)
        await finish(h, l.key, 'PASS', { ...LINUX, ...win(l.windowsRuns) });
    }
    await tick(h);
  }
  const batch = await batchOf(h, id);
  assert.equal(batch.state, 'DONE');
  assert.equal(
    batch.verdict,
    'PASS',
    JSON.stringify(batch.cells?.filter((c) => c.verdict !== 'PASS')),
  );
  assert.equal(batch.legs.length, 12);
  assert.ok(batch.legs.every((l) => l.state === 'ENDED' && l.outcome.verdict === 'PASS'));
  assert.equal(h.cloud.creates, 12);
  // Every box was destroyed by exact id after its result.
  assert.equal(h.cloud.deletes.length, 12);
  assert.equal(await h.storage.get('batch-open:' + id), undefined);
  assert.ok(batch.spentUsd > 0 && batch.spentUsd <= batch.costCapUsd);
});
test('release: RETRY re-runs on a different machine; FAIL is never re-rented; FAIL wins', async () => {
  const h = await release();
  releaseMarket(h, { twice: [...TWICE, 'TU106'] });
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  // A third TU106 machine, offered after the plan: the only fresh box for a TU106 re-run.
  const tu = h.cloud.extraOffers.find((o) => o.gpu_name === 'RTX_2060');
  h.cloud.extraOffers.push({ ...tu, id: 99, machine_id: 299099, host_id: 3999 });
  let batch = await batchOf(h, id);
  const [retry, fail] = batch.legs.filter((l) => l.state === 'STARTED');
  const sibling = (await h.storage.get('record:' + fail.key)).machineId;
  const retried = await h.storage.get('record:' + retry.key);
  await finish(h, retry.key, 'FAIL', { ...LINUX, windows_retry_media: -1 });
  await finish(h, fail.key, 'FAIL', { ...LINUX, windows_gate: -1 });
  for (const l of batch.legs.filter((x) => x.state === 'STARTED').slice(2))
    await finish(h, l.key, 'PASS', { ...LINUX, ...win(l.windowsRuns) });
  await tick(h);
  batch = await batchOf(h, id);
  const outcome = (key) => batch.legs.find((l) => l.key === key).outcome;
  assert.equal(outcome(retry.key).verdict, 'RETRY');
  assert.equal(outcome(fail.key).verdict, 'FAIL');
  const reruns = batch.legs.filter((l) => l.rerunOf);
  assert.deepEqual(
    reruns.map((l) => l.rerunOf),
    [retry.key],
  );
  assert.ok(reruns[0].avoidMachines.includes(retried.machineId));
  const rerun = await h.storage.get('record:' + reruns[0].key);
  assert.ok(rerun, 'the re-run started');
  assert.notEqual(rerun.machineId, retried.machineId);
  // Review F10: nor the machine of the die's other leg.
  assert.equal(rerun.machineId, 299099);
  assert.ok(reruns[0].avoidMachines.includes(sibling));
  assert.equal(batch.verdict, 'FAIL');
});
test('release: an owner stop-all ends the release; no leg starts afterwards', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  const stop = await h.service.fetch(
    new Request(h.bindings.WORKER_URL + '/stop-all', {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + h.bindings.OWNER_STOP_TOKEN },
    }),
  );
  assert.equal(stop.status, 200);
  await tick(h);
  await tick(h);
  const batch = await batchOf(h, id);
  assert.equal(batch.state, 'STOPPED');
  assert.equal(h.cloud.creates, 8);
  assert.ok(batch.legs.every((l) => ['ENDED', 'MISSING'].includes(l.state)));
  assert.ok(batch.legs.every((l) => !l.rerunOf));
  assert.notEqual(batch.verdict, 'PASS');
});
test('release: the batch stop closes only its own legs', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  await h.service.adminStopBatch(id);
  const batch = await batchOf(h, id);
  assert.equal(batch.state, 'STOPPED');
  assert.equal(h.cloud.deletes.length, 8);
  assert.ok(batch.legs.every((l) => l.state === 'MISSING' || l.outcome?.reason === 'STOPPED'));
});
test('start refusals: plan over the cost cap, fork commit, renting disabled, bad requests', async () => {
  const h = await release({ RELEASE_COST_CAP_USD: '2' });
  releaseMarket(h);
  await assert.rejects(h.service.adminStart({ commit_sha: sha, tier: 'large' }), (e) => {
    assert.equal(e.reason, 'PLAN_REFUSED');
    assert.deepEqual(e.plan.refusals, ['COST_CAP']);
    return true;
  });
  const fork = await release();
  releaseMarket(fork);
  fork.cloud.ancestors = [];
  await assert.rejects(
    fork.service.adminStart({ commit_sha: sha, tier: 'large' }),
    (e) => e.reason === 'NOT_ON_BRANCH',
  );
  const off = await release({ RENTING_ENABLED: 'false' });
  await assert.rejects(
    off.service.adminStart({ commit_sha: sha, tier: 'middle' }),
    (e) => e.state === 'RENTING_DISABLED',
  );
  for (const bad of [
    { commit_sha: sha },
    { commit_sha: sha, tier: 'release' },
    { commit_sha: sha, tier: 'large', die: 'AD104' },
    { commit_sha: sha, tier: 'middle', profile: 'check' },
    { commit_sha: 'A'.repeat(40), tier: 'small' },
    { commit_sha: sha, tier: 'small', die: 'ad104' },
    [],
  ])
    await assert.rejects(h.service.adminStart(bad), (e) => e.state === 'BAD_REQUEST');
  for (const x of [h, fork, off]) assert.equal(x.cloud.creates, 0);
});
test('admin middle start: one leg, the PR path checks, the commit gap applies', async () => {
  const h = await release();
  releaseMarket(h);
  // A PR rental of the commit first: an admin small/middle start is then refused by the gap.
  assert.equal((await h.call()).status, 200);
  const started = await h.service.adminStart({ commit_sha: sha, tier: 'middle' });
  assert.deepEqual(
    started.legs.map((l) => l.state),
    ['PENDING'],
  );
  const batch = await batchOf(h, started.batch);
  assert.equal(batch.legs[0].lastRefusal, 'BUDGET:sha-gap');
  assert.equal(h.cloud.creates, 1);
  // A different commit on master: admitted, one check-profile leg with the middle tier.
  const other = 'c'.repeat(40);
  h.cloud.ancestors.push(other);
  const second = await h.service.adminStart({ commit_sha: other, tier: 'small', die: 'GA106' });
  assert.equal(second.legs[0].state, 'STARTED');
  const r = await h.storage.get('record:' + second.legs[0].key);
  assert.deepEqual([r.profile, r.tier, r.die, r.family], ['check', 'small', 'GA106', 'ampere']);
  assert.equal(r.hardDeadline - r.started, 20 * 60000, 'the check profile hard limit');
  await assert.rejects(
    h.service.adminStart({ commit_sha: other, tier: 'small', die: 'GB206' }),
    (e) => e.reason === 'DIE_NOT_IN_TIER',
  );
});
test('middle tier: PRs rotate families in order and fall back when a family has no offer', async () => {
  const h = await release();
  releaseMarket(h, { drop: ['TU106', 'TU116'] });
  const families = [];
  for (let n = 1; n <= 4; n++) {
    const head = n.toString(16).padStart(2, '0').repeat(20);
    h.cloud.runs.set(String(500 + n), {
      ...h.cloud.run,
      id: 500 + n,
      head_sha: head,
      head_branch: 'pr-' + n,
      pull_requests: [{ number: n, head: { sha: head } }],
    });
    const response = await h.call('/rent', {
      claims: { run_id: String(500 + n) },
      body: {
        runner_nonce: 'f'.repeat(64),
        tested_sha: head,
        profile: 'check',
        remaining_seconds: 19800,
      },
    });
    assert.equal(response.status, 200, await response.clone().text());
    const r = await h.storage.get('record:' + (500 + n) + '-1');
    families.push([r.family, r.familyWanted ?? null, r.tier, r.selectionReason]);
    await h.service.fetch(
      new Request(h.bindings.WORKER_URL + '/stop/' + (500 + n) + '-1', {
        method: 'POST',
        headers: { Authorization: 'Bearer ' + h.bindings.OWNER_STOP_TOKEN },
      }),
    );
  }
  assert.deepEqual(families, [
    ['ampere', 'turing', 'middle', 'ROTATE'],
    ['ada', null, 'middle', 'ROTATE'],
    ['blackwell', null, 'middle', 'ROTATE'],
    ['ampere', 'turing', 'middle', 'ROTATE'],
  ]);
});
test('middle tier VRAM floor: only middle rentals, fail closed without gpu_ram', async () => {
  const h = await release({ MIDDLE_VRAM_FLOOR_MB: '16000' });
  releaseMarket(h);
  const pr = await h.call();
  assert.equal(pr.status, 503);
  assert.equal((await pr.json()).reason, 'NO_ALLOWED_OFFERS');
  // The /availability view (the runner's closed vocabulary) is unchanged by the floor.
  const view = await (await h.call('/availability?profile=check', { method: 'GET' })).json();
  assert.equal(view.rejected.VRAM, undefined);
  for (const o of h.cloud.extraOffers) o.gpu_ram = 24576;
  const ok = await release({ MIDDLE_VRAM_FLOOR_MB: '16000' });
  releaseMarket(ok);
  for (const o of ok.cloud.extraOffers) o.gpu_ram = 24576;
  assert.equal((await ok.call()).status, 200);
  // A master push is the small tier: no floor.
  const push = await release({ MIDDLE_VRAM_FLOOR_MB: '16000' });
  releaseMarket(push);
  push.cloud.run = { ...push.cloud.run, event: 'push', head_branch: 'master', pull_requests: [] };
  assert.equal((await push.call('/rent', { claims: { event_name: 'push' } })).status, 200);
});
test('every rental record carries its tier and family for the results store', async () => {
  // The store (wstore, src/ingest.js) records ledger rentals itself; it reads these fields.
  const h = await release();
  releaseMarket(h);
  assert.equal((await h.call()).status, 200);
  const pr = await h.storage.get('record:123-1');
  assert.deepEqual(
    [pr.tier, pr.family, pr.source, pr.batch],
    ['middle', 'turing', 'github', undefined],
  );
  const { batch: id } = await h.service
    .adminStart({ commit_sha: 'c'.repeat(40), tier: 'large' })
    .catch((e) => ({ e }));
  assert.equal(id, undefined, 'the fixture commit c... is not on master or next');
  h.cloud.ancestors.push('c'.repeat(40));
  // Free the PR box's machine (the only TU106 one) so the release plan is complete.
  await h.service.fetch(
    new Request(h.bindings.WORKER_URL + '/stop/123-1', {
      method: 'POST',
      headers: { Authorization: 'Bearer ' + h.bindings.OWNER_STOP_TOKEN },
    }),
  );
  const started = await h.service.adminStart({ commit_sha: 'c'.repeat(40), tier: 'large' });
  const leg = await h.storage.get('record:' + started.legs.find((l) => l.state === 'STARTED').key);
  assert.deepEqual([leg.tier, leg.source, leg.batch], ['large', 'admin', started.batch]);
  assert.equal(leg.family, 'turing');
});
test('admin DO routes: start and plan bodies are bounded JSON; refusals keep their plan', async () => {
  const object = new BrokerLedger({ storage: new MemoryStorage() }, env);
  const seen = [];
  object.service = {
    adminStart: async (body) => {
      seen.push(['start', body]);
      const e = new BrokerError('NOT_ELIGIBLE', 409, 'PLAN_REFUSED');
      e.plan = { refusals: ['COST_CAP'] };
      throw e;
    },
    adminPlan: async (body) => {
      seen.push(['plan', body]);
      return { plan: { legs: 0 }, renting: false };
    },
  };
  const post = (path, body) =>
    object.fetch(new Request('https://internal.invalid/admin/' + path, { method: 'POST', body }));
  const planned = await post('plan', JSON.stringify({ commit_sha: sha, tier: 'large' }));
  assert.equal(planned.status, 200);
  assert.deepEqual(await planned.json(), { plan: { legs: 0 }, renting: false });
  const refused = await post('start', JSON.stringify({ commit_sha: sha, tier: 'large' }));
  assert.equal(refused.status, 409);
  assert.deepEqual(await refused.json(), {
    error: 'NOT_ELIGIBLE',
    reason: 'PLAN_REFUSED',
    plan: { refusals: ['COST_CAP'] },
  });
  for (const bad of ['{', 'x'.repeat(600)]) assert.equal((await post('start', bad)).status, 400);
  const get = await object.fetch(new Request('https://internal.invalid/admin/start'));
  assert.equal(get.status, 404);
  assert.equal(seen.length, 2);
});
test('an admin leg has no GitHub run to outlive: the cron does not close it as RUN_GONE', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  // Any GitHub run lookup now says "completed"; a PR rental would be closed RUN_GONE.
  h.cloud.run = { ...h.cloud.run, status: 'completed' };
  for (const l of (await batchOf(h, id)).legs.filter((x) => x.state === 'STARTED')) {
    const r = await h.storage.get('record:' + l.key);
    await h.storage.put('record:' + l.key, {
      ...r,
      firstReportAt: h.time.value,
      lastPostAt: h.time.value,
    });
  }
  await tick(h, 1);
  const legs = (await batchOf(h, id)).legs.filter((x) => x.state === 'STARTED');
  assert.equal(legs.length, 8);
  for (const l of legs) assert.equal((await h.storage.get('record:' + l.key)).closed, null);
  assert.equal(h.cloud.deletes.length, 0);
});
test('serial legs: the second Windows leg of a die avoids the first leg machine after it ends', async () => {
  const h = await release({ MAX_CONCURRENT_RENTALS: '2' });
  releaseMarket(h, { sameHost: TWICE });
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  for (let round = 0; round < 14; round++) {
    const batch = await batchOf(h, id);
    for (const l of batch.legs.filter((x) => x.state === 'STARTED')) {
      const r = await h.storage.get('record:' + l.key);
      if (r.resultAt === undefined)
        await finish(h, l.key, 'PASS', { ...LINUX, ...win(l.windowsRuns) });
    }
    await tick(h);
  }
  const batch = await batchOf(h, id);
  assert.equal(batch.verdict, 'PASS');
  for (const die of TWICE) {
    const machines = batch.legs.filter((l) => l.die === die).map((l) => l.machineId);
    assert.equal(machines.length, 2, die);
    assert.notEqual(machines[0], machines[1], die);
  }
});
test('the release cost cap bounds every leg start, not only the plan', async () => {
  const h = await release({ RELEASE_COST_CAP_USD: '14' });
  releaseMarket(h);
  const { batch: id, estimate_usd } = await h.service.adminStart({
    commit_sha: sha,
    tier: 'large',
  });
  assert.ok(estimate_usd <= 14);
  // The market moves after the plan: the pending legs would now cost about 10 USD each.
  h.cloud.price = 2;
  for (const o of h.cloud.extraOffers) o.price = 2;
  for (const l of (await batchOf(h, id)).legs.filter((x) => x.state === 'STARTED'))
    await finish(h, l.key, 'PASS', { ...LINUX, ...win(l.windowsRuns) });
  await tick(h);
  const batch = await batchOf(h, id);
  // One more leg fits (the ended legs cost minutes); the next would pass the 14 USD cap.
  const pending = batch.legs.filter((l) => l.state === 'PENDING');
  assert.equal(h.cloud.creates, 9);
  assert.equal(pending.length, 3);
  assert.equal(pending[0].lastRefusal, 'BUDGET:release-cost-cap');
  assert.ok(batch.spentUsd <= 14, String(batch.spentUsd));
});
test('a release cell counts only its own run of the same commit', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  const [first] = (await batchOf(h, id)).legs.filter((l) => l.state === 'STARTED');
  await finish(h, first.key, 'PASS', { ...LINUX, ...win(first.windowsRuns) });
  // A record that names another commit (never written by the Worker) cannot answer the cell.
  const r = await h.storage.get('record:' + first.key);
  await h.storage.put('record:' + first.key, { ...r, sha: 'b'.repeat(40) });
  await tick(h);
  const leg = (await batchOf(h, id)).legs.find((l) => l.key === first.key);
  assert.equal(leg.outcome.verdict, 'RETRY');
  assert.equal(leg.outcome.reason, 'NOT_ENDED');
});
// Review fixes (rev259, 2026-11-05). FALSIFIERS: a release keeps renting after RENTING_ENABLED or
// RELEASE_ENABLED is turned off, or after a batch stop whose provider call failed; a contract
// price above the release cost cap keeps its box; the account-wide daily cap does not bound a
// release; the commit-gap exemption holds for a stopped release or a started leg; a pending leg
// outlives its start window; finished batches are never pruned.
async function passStarted(h, id) {
  for (const l of (await batchOf(h, id)).legs.filter((x) => x.state === 'STARTED')) {
    const r = await h.storage.get('record:' + l.key);
    if (r.resultAt === undefined)
      await finish(h, l.key, 'PASS', { ...LINUX, ...win(l.windowsRuns) });
  }
}
for (const [name, value] of [
  ['RENTING_ENABLED', 'false'],
  ['RELEASE_ENABLED', 'false'],
])
  test(
    'F1/F4: turning ' + name + ' off stops pending legs and re-runs at the next tick',
    async () => {
      const h = await release();
      releaseMarket(h);
      const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
      assert.equal(h.cloud.creates, 8);
      h.bindings[name] = value;
      await passStarted(h, id);
      await tick(h);
      await tick(h);
      const batch = await batchOf(h, id);
      assert.equal(h.cloud.creates, 8);
      assert.equal(batch.legs.filter((l) => l.state === 'MISSING').length, 4);
      assert.equal(batch.state, 'DONE');
      assert.equal(batch.verdict, 'INCOMPLETE');
    },
  );
test('F4: a release Start needs RELEASE_ENABLED=true; the plan says so', async () => {
  const h = await release({ RELEASE_ENABLED: 'false' });
  releaseMarket(h);
  const { plan } = await h.service.adminPlan({ commit_sha: sha, tier: 'large' });
  assert.deepEqual(plan.refusals, ['RELEASE_DISABLED']);
  await assert.rejects(
    h.service.adminStart({ commit_sha: sha, tier: 'large' }),
    (e) => e.state === 'RELEASE_DISABLED',
  );
  assert.equal(h.cloud.creates, 0);
});
test('F2: a batch stop whose provider calls fail still leaves the release STOPPED', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  // The stop is stored before the first provider call of the stop (read at that call).
  const seen = [];
  Object.defineProperty(h.cloud, 'vastError', {
    configurable: true,
    get: () => (seen.push(h.storage.data.get('batch:' + id).state), 500),
  });
  await assert.rejects(h.service.adminStopBatch(id));
  assert.equal(seen[0], 'STOPPED');
  Object.defineProperty(h.cloud, 'vastError', { configurable: true, writable: true, value: 0 });
  assert.equal((await batchOf(h, id)).state, 'STOPPED');
  for (let i = 0; i < 3; i++) await tick(h);
  const batch = await batchOf(h, id);
  assert.equal(batch.state, 'STOPPED');
  assert.equal(h.cloud.creates, 8);
  assert.equal(h.cloud.deletes.length, 8, 'the cron finishes the stop by exact id');
});
test('F8: a verified contract price that breaks the release cost cap closes the box', async () => {
  const h = await release({ RELEASE_COST_CAP_USD: '14' });
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  const [leg] = (await batchOf(h, id)).legs.filter((l) => l.state === 'STARTED');
  const r = await h.storage.get('record:' + leg.key);
  const instance = h.cloud.instances.get(String(r.id));
  instance.dph_total = 1.9;
  instance.dph_base = 1.9 - (instance.storage_cost * 120) / 720;
  await tick(h, 1);
  const after = await h.storage.get('record:' + leg.key);
  assert.equal(after.closed, 'CONTRACT_BUDGET');
  assert.ok(h.cloud.deletes.includes(String(r.id)));
});
test('F8: the account-wide daily cap bounds every profile together', async () => {
  const h = await release({ ACCOUNT_DAILY_USD: '3' });
  releaseMarket(h);
  const started = await h.service.adminStart({ commit_sha: sha, tier: 'large' }).catch((e) => e);
  assert.deepEqual(started.plan?.refusals, ['ACCOUNT_DAILY_USD']);
  assert.equal(h.cloud.creates, 0);
});
test('F9: the gap exemption needs a RUNNING large batch and a PENDING leg of it', async () => {
  const { Ledger } = await import('../src/ledger.js');
  const ledger = new Ledger(new MemoryStorage());
  const batch = {
    id: '8' + '1'.repeat(18),
    sha,
    tier: 'large',
    state: 'RUNNING',
    legs: [
      { key: '8' + '1'.repeat(18) + '-1', state: 'PENDING' },
      { key: '8' + '1'.repeat(18) + '-2', state: 'STARTED' },
    ],
  };
  const check = async (patch, key = batch.legs[0].key) => {
    await ledger.put('batch:' + batch.id, { ...batch, ...patch });
    return ledger.batchLeg(batch.id, key, sha);
  };
  assert.equal(await check({}), true);
  assert.equal(await check({ state: 'STOPPED' }), false);
  assert.equal(await check({ state: 'DONE' }), false);
  assert.equal(await check({}, batch.legs[1].key), false);
  assert.equal(await check({ tier: 'middle' }), false);
  assert.equal(await check({ sha: 'b'.repeat(40) }), false);
});
test('F9: a pending leg past its start window is MISSING, not rented later', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  // The last pending leg's die (GB206) leaves the market after the plan.
  h.cloud.extraOffers = h.cloud.extraOffers.filter((o) => o.gpu_name !== 'RTX_5060');
  await passStarted(h, id);
  await tick(h);
  let gb = (await batchOf(h, id)).legs.find((l) => l.die === 'GB206');
  assert.equal(gb.state, 'PENDING');
  await passStarted(h, id);
  await tick(h, 361);
  gb = (await batchOf(h, id)).legs.find((l) => l.die === 'GB206');
  assert.equal(gb.state, 'MISSING');
});
test('F10: finished batches are pruned after 30 days; legs use a non-git branch name', async () => {
  const h = await release();
  releaseMarket(h);
  const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
  const leg = (await batchOf(h, id)).legs.find((l) => l.state === 'STARTED');
  assert.equal((await h.storage.get('record:' + leg.key)).branch, 'batch:' + id);
  await h.service.adminStopBatch(id);
  await tick(h);
  assert.ok(await batchOf(h, id));
  await tick(h, 31 * 24 * 60);
  assert.equal(await batchOf(h, id), undefined);
});
test('F8: the account-wide daily cap also refuses a single PR rental (closed reason vocabulary)', async () => {
  const h = await release({ ACCOUNT_DAILY_USD: '0.05' });
  releaseMarket(h);
  const pr = await h.call();
  assert.equal(pr.status, 409);
  const body = await pr.json();
  assert.equal(body.reason, 'BUDGET:daily-usd');
  assert.equal(body.budget.daily_cap_usd, 0.05);
  assert.equal(h.cloud.creates, 0);
});

// Integration of #258 (result store) with this tier work (review rev259 F7, 2026-11-05).
// FALSIFIER: a release leg reaches D1 as tier middle/adhoc or profile check, a leg is recorded
// twice, or a leg whose box reported another die is stored as PASS.
test(
  'release + store: every leg is one D1 row, tier largest; a wrong-die PASS is UNATTRIBUTED',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1();
    const h = await release({ DB: db, ROW_TAG_KEY: 't'.repeat(48) });
    releaseMarket(h);
    await h.storage.put('treekey:' + sha, { key: 'f'.repeat(64), kind: 'tree', at: h.time.value });
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    const started = (await batchOf(h, id)).legs.filter((l) => l.state === 'STARTED');
    assert.equal(started.length, 8);
    const [wrong, ...rest] = started;
    const other = (await h.storage.get('record:' + wrong.key)).die === 'AD102' ? 'AD104' : 'AD102';
    await finish(h, wrong.key, 'PASS', { ...LINUX, ...win(wrong.windowsRuns) }, boxHardware(other));
    for (const l of rest) await finish(h, l.key, 'PASS', { ...LINUX, ...win(l.windowsRuns) });
    for (let i = 0; i < 4; i++) await tick(h);
    const rows = db.rows(
      'SELECT run_id, tier, profile, platform, verdict FROM ci_runs ORDER BY run_id',
    );
    const finals = rows.filter((r) => started.some((l) => r.run_id === 'vast-' + l.key + '-0'));
    assert.equal(finals.length, 8, JSON.stringify(rows));
    assert.equal(new Set(rows.map((r) => r.run_id)).size, rows.length, 'one row per leg');
    for (const r of finals) {
      assert.deepEqual([r.tier, r.profile, r.platform], ['largest', 'release', 'both']);
      assert.equal(
        r.verdict,
        r.run_id === 'vast-' + wrong.key + '-0' ? 'UNATTRIBUTED' : 'PASS',
        r.run_id,
      );
    }
  },
);

// Review of PR #269, R2 (2026-11-05). FALSIFIER: one fully passing release (two dies per family,
// some dies on one machine with two Windows runs, others on two machines) does not meet the
// committed `large` requirements in the store.
test(
  'release + store: one passing release meets the committed large requirements',
  { skip: d1Skip },
  async () => {
    const db = new FakeD1();
    const tagKey = 't'.repeat(48);
    const h = await release({ DB: db, ROW_TAG_KEY: tagKey });
    releaseMarket(h);
    await h.storage.put('treekey:' + sha, { key: 'f'.repeat(64), kind: 'tree', at: h.time.value });
    const requirements = readFileSync(new URL('../../requirements.toml', import.meta.url), 'utf8');
    const { batch: id } = await h.service.adminStart({ commit_sha: sha, tier: 'large' });
    for (let round = 0; round < 3; round++) {
      const batch = await batchOf(h, id);
      for (const l of batch.legs.filter((x) => x.state === 'STARTED')) {
        const r = await h.storage.get('record:' + l.key);
        if (r.resultAt === undefined)
          await finish(h, l.key, 'PASS', { ...LINUX, ...win(l.windowsRuns) });
      }
      await tick(h);
    }
    for (let i = 0; i < 3; i++) await tick(h);
    const batch = await batchOf(h, id);
    assert.equal(batch.verdict, 'PASS');
    assert.ok(
      batch.legs.some((l) => l.windowsRuns === 2),
      'a one-machine die ran Windows twice',
    );
    const env = { DB: db, ROW_TAG_KEY: tagKey };
    assert.equal(db.rows('SELECT run_id FROM ci_runs WHERE verdict = ?', 'PASS').length, 12);
    const answer = await requirementsMet(env, {
      commit_sha: sha,
      tree_hash: null,
      tier: 'large',
      requirements,
      now: h.time.value,
    });
    assert.deepEqual(answer, { met: true, missing: [], inflight: [] });
    // (One Windows run too few per die is covered in requirements.test.js: a stage row deleted
    // here would break the row's HMAC tag and answer TAMPERED instead.)
  },
);
