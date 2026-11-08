import test from 'node:test';
import assert from 'node:assert/strict';
import { plan, validatePlan, validateAssignment, withoutSeeds, PlanError } from '../index.js';
import { fixture, clone } from './helpers.js';
import { seeded } from './seed-fixture.js';
// FALSIFIER: seeding creates a correctness dependency, ignores its upload bill,
// increases planned box minutes, or cannot safely fall back after its deadline.
for (const n of [1, 2])
  test('seeder role re-ranks cheap compute with expensive upload: ' + n, () => {
    const f = seeded(n),
      p = plan(f);
    assert.equal(p.admitted, true);
    const a = p.assignments.find((a) => a.die === 'TU106');
    assert.equal(a.box_id, 'offer-1');
    assert.equal(a.assigned_consumers.length, n);
    assert.ok(Math.abs(a.pricing.bandwidth_up_gb - (2 + n * 13.1)) < 1e-6);
    for (const c of p.assignments.filter((a) => a.tier === 'medium')) {
      assert.equal(c.pricing.bandwidth_up_gb, 2);
      assert.equal(c.seed_hints.length, 2);
    }
    validatePlan(f, p);
  });
for (const n of [1, 2])
  test('removing seed edges preserves exact coverage and raises box minutes: ' + n, () => {
    const f = seeded(n),
      p = plan(f),
      cold = withoutSeeds(f, p);
    assert.ok(p.assignments.some((a) => a.seed_hints.length));
    assert.deepEqual(cold.coverage, p.coverage);
    assert.ok(cold.total_box_minutes > p.total_box_minutes);
    assert.ok(cold.total_reserved_usd < p.total_reserved_usd);
    assert.ok(
      cold.assignments.every((a) => a.seed_hints.length === 0 && a.assigned_consumers.length === 0),
    );
    validatePlan(f, cold);
  });
for (const n of [1, 2])
  test('deadline and fallback reserve cover cold builds: ' + n, () => {
    const f = seeded(n),
      p = plan(f);
    for (const a of p.assignments.filter((a) => a.seed_hints.length)) {
      assert.equal(a.worst_case_minutes, a.fallback_minutes + f.limits.seeding.max_wait_minutes);
      for (const h of a.seed_hints) {
        assert.equal(h.deadline_minute, a.start_minute + 5);
        assert.ok(h.fallback_step_ids.every((id) => a.runs.some((r) => r.id === id)));
        assert.ok(h.transfer_minutes <= 5);
      }
      assert.throws(
        () => validateAssignment(f, a),
        (e) => e.code === 'SEED_CONTEXT',
      );
      validateAssignment(f, a, { plan: p });
    }
  });
for (const n of [1, 2])
  test('upload reservation appears in total and per-tier quotes: ' + n, () => {
    const f = seeded(n),
      p = plan(f),
      a = p.assignments.find((a) => a.assigned_consumers.length);
    const expected =
      a.pricing.bandwidth_down_gb *
        f.boxes.find((b) => b.id === a.box_id).pricing.inet_down_usd_gb +
      a.pricing.bandwidth_up_gb * f.boxes.find((b) => b.id === a.box_id).pricing.inet_up_usd_gb;
    assert.ok(Math.abs(a.pricing.transfer_usd - expected) < 1e-6);
    assert.ok(
      Math.abs(
        p.total_reserved_usd - p.assignments.reduce((s, a) => s + a.pricing.reserved_usd, 0),
      ) < 1e-6,
    );
    assert.ok(
      Math.abs(p.total_reserved_usd - p.tier_totals.reduce((s, a) => s + a.reserved_usd, 0)) < 1e-6,
    );
  });
for (const n of [1, 2])
  test('seeds never turn capped cold plan into an over-cap rental: ' + n, () => {
    const f = seeded(n);
    f.limits.seeding.enabled = false;
    const cold = plan(f);
    f.limits.cost_cap_usd = cold.total_reserved_usd;
    f.limits.seeding.enabled = true;
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.total_reserved_usd <= f.limits.cost_cap_usd);
    validatePlan(f, p);
  });
for (const n of [1, 2])
  test('insufficient fallback headroom disables optional seeding: ' + n, () => {
    const f = seeded(n);
    f.limits.seeding.max_wait_minutes = 250;
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.seed_hints.length === 0));
  });
for (const mutation of ['deadline', 'fallback', 'consumer', 'source', 'upload'])
  for (const n of [1, 2])
    test('reject tampered seed ' + mutation + ' ' + n, () => {
      const f = seeded(n),
        p = plan(f),
        a = p.assignments.find((a) => a.seed_hints.length),
        h = a.seed_hints[0];
      if (mutation === 'deadline') h.deadline_minute += 100;
      if (mutation === 'fallback') h.fallback_step_ids = [];
      if (mutation === 'consumer')
        p.assignments.find((a) => a.assigned_consumers.length).assigned_consumers = [];
      if (mutation === 'source') h.producer_box_id = 'foreign';
      if (mutation === 'upload')
        p.assignments.find((a) => a.assigned_consumers.length).pricing.bandwidth_up_gb = 2;
      assert.throws(
        () => validatePlan(f, p),
        (e) => e instanceof PlanError,
      );
    });
for (const n of [1, 2])
  test('slow upload declines seed hints but preserves a cold-valid plan ' + n, () => {
    const f = seeded(n);
    f.boxes.forEach((b) => (b.limits.inet_up_mbps = 50));
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.seed_hints.every((h) => h.artifact !== 'open_image')));
    validatePlan(f, p);
  });
for (const n of [1, 2])
  test('seeder upload-speed preference is data ' + n, () => {
    const f = seeded(n);
    f.boxes[0].pricing.inet_up_usd_gb = 0.001;
    f.boxes[1].pricing.compute_usd_hour = 0.101;
    f.boxes[1].limits.inet_up_mbps = 5000;
    f.limits.seeding.upload_cost_weight = 0;
    const cheap = plan(f);
    assert.equal(cheap.assignments.find((a) => a.die === 'TU106').box_id, 'offer-0');
    f.limits.seeding.upload_speed_weight = 0.01;
    const fast = plan(f);
    assert.equal(fast.assignments.find((a) => a.die === 'TU106').box_id, 'offer-1');
  });
for (const n of [1, 2])
  test('export deadlines and publication offsets are bound to assignments ' + n, () => {
    const f = seeded(n),
      p = plan(f),
      source = p.assignments.find((a) => a.seed_exports.length);
    assert.ok(
      source.seed_exports.every((e) =>
        source.runs.some((r) => r.id === e.ready_after_step_id && r.produces.includes(e.scope_key)),
      ),
    );
    assert.equal(
      source.worst_case_minutes,
      source.fallback_minutes + source.seed_exports.length * f.limits.seeding.max_wait_minutes,
    );
    source.seed_exports[0].available_minute += n;
    assert.throws(
      () => validatePlan(f, p),
      (e) => e instanceof PlanError,
    );
  });
for (const n of [1, 2])
  test('consumer deadline needs full cold-build headroom ' + n, () => {
    const f = seeded(n);
    f.limits.seeding.max_wait_minutes = 30;
    for (const q of f.requirements.items.filter((q) => q.id.startsWith('consumer-')))
      q.runs = [
        'linux_apps_full',
        'windows_apps_full',
        'windows_gate',
        'windows_gate',
        'windows_gate',
      ];
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.seed_hints.length === 0));
  });
for (const n of [1, 2])
  test('source publication deadlines fit source hard budget ' + n, () => {
    const f = seeded(n);
    f.limits.seeding.max_wait_minutes = 20;
    f.requirements.items[0].runs = [
      'linux_image_build',
      'kayfabe_build',
      'windows_apps_full',
      ...Array(4).fill('windows_gate'),
    ];
    const p = plan(f);
    assert.equal(p.admitted, true);
    const source = p.assignments.find((a) => a.die === 'TU106');
    assert.equal(source.seed_exports.length, 1);
    assert.equal(source.worst_case_minutes, 275);
  });
