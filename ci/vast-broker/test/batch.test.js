import test from 'node:test';
import assert from 'node:assert/strict';
import { BrokerLedger } from '../src/index.js';
import { BrokerError } from '../src/policy.js';
import { setup, sha, env, MemoryStorage } from './helpers.js';
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
const release = (extra = {}) =>
  setup({ MAX_CONCURRENT_RENTALS: '8', ROLLING_SPEND_USD: '200', ...extra });
const creates = (h) =>
  h.cloud.calls.filter((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url)).map((c) => c.body);
const LINUX = { provision: 1, gates: 9, thin_suite: 30, cuda_host: 4, cuda_fat: 4 };
const hardware = { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' };
async function finish(h, key, verdict, spec) {
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
        hardware,
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
    assert.equal(r.profile, 'release');
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
  // The release holds the commit gap: a PR rental of the same commit is refused.
  const pr = await h.call();
  assert.equal(pr.status, 409);
  assert.equal((await pr.json()).reason, 'BUDGET:sha-gap');
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
  let batch = await batchOf(h, id);
  const [retry, fail] = batch.legs.filter((l) => l.state === 'STARTED');
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
  const h = await release({ MAX_CONCURRENT_RENTALS: '1' });
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
