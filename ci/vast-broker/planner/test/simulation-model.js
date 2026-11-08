import assert from 'node:assert/strict';
import { plan, validatePlan, withoutSeeds } from '../index.js';
import { fixture, clone } from './helpers.js';
import { MockCloud, MemoryStorage, clock } from '../../test/helpers.js';
import { Ledger } from '../../src/ledger.js';
import { Vast } from '../../src/vast.js';
import { IMAGE } from '../../src/policy.js';
import { sha256Hex } from '../../src/tokens.js';
export function random(seed) {
  let n = seed >>> 0;
  return () => {
    n += 0x6d2b79f5;
    let t = n;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
export function market(seed) {
  const f = fixture(),
    rng = random(seed),
    pick = (xs) => xs[Math.floor(rng() * xs.length)],
    base = clone(f.boxes[0]),
    driver = clone(f.requirements.items[0].driver);
  const dies = [
    ['TU106', 'Turing'],
    ['TU116', 'Turing'],
    ['GA102', 'Ampere'],
    ['GA104', 'Ampere'],
    ['AD102', 'Ada'],
    ['AD104', 'Ada'],
    ['GB202', 'Blackwell'],
    ['GB205', 'Blackwell'],
  ];
  f.catalog.dies = dies.map(([id, die_family]) => ({
    id,
    die_family,
    status: 'executable',
    supported_branches: [580],
    minimum_driver_branches: {
      host: die_family === 'Blackwell' ? 570 : 580,
      linux_guest: die_family === 'Blackwell' ? 570 : 580,
      windows_guest: die_family === 'Blackwell' ? 570 : 580,
    },
  }));
  f.catalog.driver_sets[0].supported_dies = dies.map((d) => d[0]);
  f.requirements.items = [];
  f.boxes = [];
  for (const [d, [die, die_family]] of dies.entries()) {
    f.requirements.items.push({
      id: 'primary-' + die,
      die,
      driver,
      tier: d % 2 ? 'medium' : 'large',
      evidence: 'primary',
      must: true,
      runs:
        seed % 4 === 0
          ? ['linux_apps_full', 'windows_gate']
          : seed % 4 === 1
            ? ['merge_bar', 'windows_gate']
            : seed % 4 === 2
              ? ['windows_gate']
              : ['linux_apps_full'],
    });
    for (let i = 0; i < 8; i++) {
      const id = 10000 + d * 100 + i,
        b = clone(base);
      Object.assign(b, {
        id: String(id),
        machine_id: String(20000 + d * 100 + i),
        host_id: String(30000 + Math.floor(rng() * 20)),
        die,
        country: pick(['US', 'CA', 'DE', 'NL', null]),
        provider: 'vast',
      });
      Object.assign(b.pricing, {
        compute_usd_hour: 0.04 + rng() * 0.6,
        inet_down_usd_gb: pick([0, 0.001, 0.003, 0.005, 0.01, 0.03]),
        inet_up_usd_gb: pick([0.001, 0.003, 0.006, 0.02]),
        storage_usd_gb_month: pick([0, 0.08, 0.12, 0.2]),
      });
      Object.assign(b.limits, {
        cpu_cores: pick([4, 8, 12, 24]),
        disk_gb: pick([100, 120, 220, 300, 500]),
        disk_mb_per_second: pick([50, 100, 200, 500]),
        inet_down_mbps: pick([100, 200, 445, 1000, 3000]),
        inet_up_mbps: pick([20, 50, 100, 1000]),
        reliability: pick([0.9, 0.98, 0.999]),
        dlperf_ratio: pick([0.2, 0.5, 1, 2]),
      });
      b.capabilities.kvm = rng() > 0.12;
      b.capabilities.vfio = rng() > 0.12;
      if (i === 0) {
        b.pricing = { ...clone(base.pricing), compute_usd_hour: 0.12 + d * 0.001 };
        b.limits = clone(base.limits);
        b.capabilities = clone(base.capabilities);
      } else if (rng() < 0.15)
        b.pricing[pick(['inet_up_usd_gb', 'compute_usd_hour', 'storage_usd_gb_month'])] = pick([
          null,
          '0.001',
          -1,
          Number.NaN,
        ]);
      f.boxes.push(b);
    }
  }
  if (seed % 9 === 0) {
    const die = dies[seed % 8][0];
    f.boxes = f.boxes.filter((b) => b.die !== die);
  }
  if (seed % 3 === 0) f.limits.bad_machine_ids = [f.boxes.at(-1)?.machine_id].filter(Boolean);
  if (seed % 4 === 0) f.limits.bad_host_ids = ['30019'];
  if (seed % 5 === 0) {
    const b = clone(f.boxes[0]);
    b.id = '99000';
    b.pricing.compute_usd_hour = 0.001;
    f.boxes.push(b);
  }
  f.limits.max_concurrent_boxes = 3 + (seed % 6);
  f.limits.max_boxes = 32;
  f.limits.cost_cap_usd = seed % 5 === 0 ? 5 : 20 + (seed % 26);
  f.limits.seeding.enabled = true;
  return f;
}
export function oracle(f, p) {
  assert.ok(p.total_reserved_usd <= f.limits.cost_cap_usd, 'total reservation cap');
  assert.equal(
    new Set(p.assignments.map((a) => a.machine_id)).size,
    p.assignments.length,
    'one physical machine per assignment',
  );
  const boxes = new Map(f.boxes.map((b) => [b.id, b])),
    profiles = new Map(f.catalog.profiles.map((p) => [p.tier, p]));
  for (const a of p.assignments) {
    const b = boxes.get(a.box_id),
      r = profiles.get(a.tier),
      q = b.pricing;
    assert.ok(
      !f.limits.bad_machine_ids.includes(b.machine_id) &&
        !f.limits.bad_host_ids.includes(b.host_id),
    );
    assert.ok(
      q.inet_down_usd_gb <= r.max_inet_usd_gb && q.inet_up_usd_gb <= r.max_inet_usd_gb,
      'per-GB guards',
    );
    const hourly =
      q.compute_usd_hour +
      (q.storage_usd_gb_month * a.pricing.disk_gb) / f.limits.storage_month_hours;
    const transfer =
      a.pricing.bandwidth_down_gb * q.inet_down_usd_gb +
      a.pricing.bandwidth_up_gb * q.inet_up_usd_gb;
    const raw = (hourly * a.reservation_minutes) / 60 + transfer;
    assert.ok(
      a.pricing.reserved_usd >= raw - 1e-9 &&
        a.pricing.reserved_usd - raw <= (a.reservation_minutes / 60 + 2) * 1e-6,
      'independent conservative reserve arithmetic',
    );
    assert.ok(
      a.pricing.reserved_usd + 1e-9 >=
        (a.pricing.hourly_usd * a.reservation_minutes) / 60 + a.pricing.transfer_usd,
      'published quote self consistency',
    );
    assert.ok(
      a.pricing.effective_usd_hour <= f.limits.price_caps.find((p) => p.tier === a.tier).usd_hour,
      'per-tier price cap',
    );
    assert.ok(
      a.worst_case_minutes <=
        Math.min(b.limits.hard_minutes, f.limits.hard_minutes) - f.limits.margin_minutes + 1e-7,
      'fallback time fits',
    );
    for (const s of a.runs)
      for (const cap of f.catalog.runs.find((r) => r.id === s.run_id).capabilities)
        assert.equal(b.capabilities[cap], true);
    const at = a.start_minute;
    assert.ok(
      p.assignments.filter(
        (b) => b.start_minute <= at && b.start_minute + b.worst_case_minutes > at,
      ).length <= f.limits.max_concurrent_boxes,
      'worst-case concurrency',
    );
  }
  const expected = [];
  for (const q of f.requirements.items)
    for (const d of q.driver ? ['primary'] : (q.driver_sweep.branches ?? q.driver_sweep.patches))
      for (const [i, id] of q.runs.entries()) expected.push([q.id, d, i, id].join('/'));
  assert.deepEqual(
    p.coverage
      .map((c) => [c.requirement_id, c.driver_set_id, c.run_index, c.run_id].join('/'))
      .sort(),
    expected.sort(),
    'exact requirement multiset',
  );
  if (p.assignments.some((a) => a.seed_hints.length)) {
    const cold = withoutSeeds(f, p);
    assert.deepEqual(cold.coverage, p.coverage);
    assert.ok(
      cold.total_box_minutes >= p.total_box_minutes - 1e-6,
      'edge removal cannot increase optimization savings',
    );
    assert.ok(
      cold.assignments.every((a) => a.seed_hints.length === 0 && a.seed_exports.length === 0),
    );
  } else assert.ok(p.assignments.every((a) => a.seed_exports.length === 0));
}
// Simulation-only execution adapter over the repo's real MockCloud/Vast/Ledger.
// No assignment routes or Python execution is claimed by this test harness.
export async function execute(f, p, { fault = null } = {}) {
  const cloud = new MockCloud(),
    storage = new MemoryStorage();
  let ledger = new Ledger(storage),
    client = new Vast(cloud.fetch, {}, 'mock-vast-simulation');
  const owned = new Set(),
    records = [],
    recordByAssignment = new Map();
  cloud.contracts = Array.from({ length: 200 }, (_, i) => 900 + i);
  cloud.extraOffers = p.assignments.map((a, i) => {
    const b = f.boxes.find((b) => b.id === a.box_id);
    return {
      ...cloud.offer,
      id: 10000 + i,
      machine_id: Number(b.machine_id) || 100 + i,
      storage_cost: b.pricing.storage_usd_gb_month,
      inet_down_cost: b.pricing.inet_down_usd_gb,
      inet_up_cost: b.pricing.inet_up_usd_gb,
      price:
        b.pricing.compute_usd_hour + (b.pricing.storage_usd_gb_month * a.pricing.disk_gb) / 720,
    };
  });
  if (!p.admitted) {
    assert.equal(cloud.creates, 0);
    return { cloud, records, ledger, owned };
  }
  validatePlan(f, p);
  const events = p.assignments
    .flatMap((a, i) => [
      { kind: 'start', at: a.start_minute, a, i },
      { kind: 'end', at: a.start_minute + a.worst_case_minutes, a, i },
    ])
    .sort(
      (a, b) =>
        a.at - b.at ||
        Number(a.kind === 'start') - Number(b.kind === 'start') ||
        a.a.id.localeCompare(b.a.id),
    );
  let aborted = false,
    reserved = 0,
    injected = false,
    fallbacks = 0;
  for (const event of events) {
    const { a, i } = event,
      now = clock + Math.round(event.at * 60000);
    if (event.kind === 'start') {
      if (aborted) continue;
      const key = String(5000 + i) + '-1',
        script = 'mock-assignment:' + a.id;
      const r = {
        key,
        run: String(5000 + i),
        attempt: '1',
        sha: f.source_revision,
        label: 'kf-ci-' + key,
        scriptHash: await sha256Hex(script),
        started: now,
        ended: null,
        id: null,
        idSource: 'create',
        profile: 'check',
        budgetClass: 'release',
        price: a.pricing.hourly_usd,
        computeStoragePrice: a.pricing.hourly_usd,
        bandwidthCost: a.pricing.transfer_usd,
        ttl: a.reservation_minutes * 60000,
        billingTtl: a.reservation_minutes * 60000,
      };
      await ledger.save(r);
      await storage.put('open:' + r.key, true);
      records.push(r);
      recordByAssignment.set(a.id, r);
      const countBefore = cloud.created.length;
      try {
        if (
          (fault === 'partial-five' && cloud.creates === 5) ||
          (fault === 'offer-disappears' && !injected)
        ) {
          injected = true;
          throw Error('synthetic create refusal');
        }
        if (fault === 'ambiguous-create' && !injected) {
          cloud.ambiguous = true;
          injected = true;
        }
        const response = await cloud.fetch(
          'https://console.vast.ai/api/v0/asks/' + (10000 + i) + '/',
          {
            method: 'PUT',
            body: JSON.stringify({
              image: IMAGE,
              vm: true,
              disk: a.pricing.disk_gb,
              label: r.label,
              onstart: script,
            }),
          },
        );
        const answer = await response.json();
        assert.equal(answer.success, true);
        r.id = String(answer.new_contract);
      } catch (error) {
        if (cloud.created.length > countBefore) {
          cloud.ambiguous = false;
          ledger = new Ledger(storage);
          client = new Vast(cloud.fetch, {}, 'mock-vast-simulation');
          r.id = await client.reconcile(r);
          assert.ok(r.id);
          r.idSource = 'reconcile';
        } else {
          r.closed = 'GONE';
          await ledger.settle(r, now);
          aborted = true;
        }
      } finally {
        for (const row of cloud.created.slice(countBefore)) owned.add(String(row.contract));
      }
      if (r.id) {
        await ledger.save(r);
        reserved += a.pricing.reserved_usd;
        assert.ok(reserved <= f.limits.cost_cap_usd + 1e-6);
        const instance = cloud.instances.get(r.id),
          hourly = instance.dph_base + (instance.storage_cost * a.pricing.disk_gb) / 720;
        assert.ok(
          Math.abs(hourly - a.pricing.hourly_usd) <= 0.000002,
          'live mocked storage allocation matches quote',
        );
        assert.ok(
          instance.inet_down_cost <=
            f.catalog.profiles.find((p) => p.tier === a.tier).max_inet_usd_gb &&
            instance.inet_up_cost <=
              f.catalog.profiles.find((p) => p.tier === a.tier).max_inet_usd_gb,
        );
        if (fault === 'offline-once' && !injected) {
          injected = true;
          instance.actual_status = 'offline';
          assert.ok(await client.inspect(r));
          assert.equal((await storage.get('record:' + r.key)).ended, null);
          instance.actual_status = 'running';
        }
        if (fault === 'api-500' || fault === 'api-429' || fault === 'api-timeout') {
          if (!injected) {
            injected = true;
            const broken = new Vast(
              async (url, init) => {
                if (fault === 'api-timeout') throw new DOMException('mock timeout', 'TimeoutError');
                return new Response('{}', {
                  status: fault === 'api-500' ? 500 : 429,
                  headers: { 'Retry-After': '120' },
                });
              },
              {},
              'mock-vast-simulation',
            );
            await assert.rejects(() => broken.inspect(r));
            assert.equal((await storage.get('record:' + r.key)).ended, null);
          }
        }
        if (fault === 'third-party' && !injected) {
          injected = true;
          cloud.instances.delete(r.id);
        }
        if (fault === 'restart') {
          ledger = new Ledger(storage);
          client = new Vast(cloud.fetch, {}, 'mock-vast-simulation');
        }
        if (fault === 'seeder-dies' && a.seed_hints.length) {
          const producer = recordByAssignment.get(a.seed_hints[0].producer_assignment_id);
          assert.ok(
            producer && cloud.instances.has(producer.id),
            'seeder still active when consumer starts',
          );
          cloud.instances.delete(producer.id);
          fallbacks++;
          const cold = withoutSeeds(f, p);
          assert.deepEqual(cold.coverage, p.coverage);
          assert.ok(
            a.fallback_minutes + f.limits.seeding.max_wait_minutes <=
              a.reservation_minutes - f.limits.margin_minutes,
          );
        }
      }
      assert.ok(
        (await ledger.openRecords()).length <= f.limits.max_concurrent_boxes,
        'ledger concurrent intents',
      );
    } else {
      const r = recordByAssignment.get(a.id);
      if (!r || r.ended !== null) continue;
      await client.destroy(r);
      assert.equal(cloud.instances.has(r.id), false);
      r.closed = 'STOPPED';
      await ledger.settle(r, now);
    }
  }
  assert.equal((await ledger.openRecords()).length, 0);
  assert.equal(cloud.instances.size, 2, 'only foreign fixtures remain');
  assert.ok(cloud.deletes.every((id) => owned.has(id)));
  assert.ok([...owned].every((id) => !cloud.instances.has(id)));
  assert.ok(records.every((r) => Number.isFinite(r.ended) && r.verdict !== 'PASS'));
  return { cloud, records, ledger, owned, fallbacks };
}
