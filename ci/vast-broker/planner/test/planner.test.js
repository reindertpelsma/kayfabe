import test from 'node:test';
import assert from 'node:assert/strict';
import {
  plan,
  validatePlan,
  validateAssignment,
  resolveDependencies,
  withoutSeeds,
  PlanError,
} from '../index.js';
import { fixture, clone } from './helpers.js';
const rejects = (fn, code) =>
  assert.throws(fn, (e) => e instanceof PlanError && (!code || e.code === code));
// FALSIFIER: one omitted/duplicated requirement, missing setup, fake quote or unsafe
// driver transition can become an admitted plan or a valid executor assignment.
for (const provider of ['vast', 'owned-workstation'])
  test('complete plan and self-run assignment: ' + provider, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.provider = provider));
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.equal(p.assignments.length, 1);
    assert.deepEqual(validatePlan(f, p), { valid: true, executable: true });
    assert.deepEqual(validateAssignment(f, p.assignments[0]), { valid: true, executable: true });
    assert.equal(p.assignments[0].provider, provider);
    assert.equal(p.coverage.length, 4);
    assert.ok(p.coverage.every((c) => c.status === 'covered'));
  });
for (const reordered of [false, true])
  test('pure and independent of collection/object order: ' + reordered, () => {
    const f = fixture(),
      before = clone(f),
      p = plan(f);
    assert.deepEqual(f, before);
    const g = clone(f);
    for (const key of ['runs', 'artifacts', 'dies', 'driver_pins', 'driver_sets', 'profiles'])
      g.catalog[key].reverse();
    g.requirements.items.reverse();
    g.boxes.reverse();
    g.limits.price_caps.reverse();
    if (reordered) g.boxes = g.boxes.map((b) => Object.fromEntries(Object.entries(b).reverse()));
    assert.deepEqual(plan(g), p);
  });
for (const run of ['spec_lane_a', 'windows_apps_full'])
  test('pack overflow onto another physical machine: ' + run, () => {
    const f = fixture();
    f.requirements.items[0].runs.push(run, run);
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.length > 1);
    assert.equal(new Set(p.assignments.map((a) => a.machine_id)).size, p.assignments.length);
    for (const a of p.assignments) assert.ok(a.worst_case_minutes <= 285);
    validatePlan(f, p);
  });
for (const count of [1, 2])
  test('shared setup charged once; repeated gate preserved: ' + count, () => {
    const f = fixture();
    f.requirements.items[0].runs = Array(count).fill('windows_gate');
    const p = plan(f),
      a = p.assignments[0];
    assert.equal(a.runs.filter((r) => r.run_id === 'windows_install_gate').length, 1);
    assert.equal(a.runs.filter((r) => r.run_id === 'windows_gate').length, count);
    assert.equal(a.fallback_minutes, 10 + 33 + 17 * count);
  });
for (const run of ['guest_linux_tiny', 'guest_windows_tiny'])
  test('driver subrun reuses compatible setup: ' + run, () => {
    const f = fixture();
    f.requirements.items.push({
      ...clone(f.requirements.items[0]),
      id: 'sweep',
      evidence: 'driver',
      driver_sweep: { branches: ['580'] },
      runs: [run],
    });
    delete f.requirements.items[1].driver;
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.equal(p.assignments.length, 1);
    validatePlan(f, p);
  });
for (const count of [1, 2])
  test('host swaps isolated from primary evidence: ' + count, () => {
    const f = fixture();
    f.requirements.items.push({
      ...clone(f.requirements.items[0]),
      id: 'host-sweep',
      evidence: 'driver',
      runs: Array(count).fill('host_tiny'),
    });
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.length >= 2);
    for (const a of p.assignments)
      if (a.runs.some((r) => r.run_id === 'host_tiny')) assert.equal(a.primary_evidence, false);
    validatePlan(f, p);
  });
for (const capability of ['kvm', 'can_swap_host_driver'])
  test('capabilities cannot be borrowed from another provider: ' + capability, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.capabilities[capability] = false));
    if (capability === 'can_swap_host_driver') {
      f.requirements.items[0].runs = ['host_tiny'];
      f.requirements.items[0].evidence = 'driver';
    }
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.ok(
      p.assignments.every((a) =>
        a.runs.every(
          (r) => !f.catalog.runs.find((x) => x.id === r.run_id).capabilities.includes(capability),
        ),
      ),
    );
  });
for (const die of ['TU106', 'TU116'])
  test('die not offered is explicit and never failure: ' + die, () => {
    const f = fixture();
    f.requirements.items[0].die = die;
    f.boxes = f.boxes.filter((b) => b.die !== die);
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.ok(p.coverage.every((c) => c.status === 'not_offered'));
    assert.equal(p.assignments.length, 0);
    assert.ok(!JSON.stringify(p).includes('FAIL'));
    validatePlan(f, p, { forExecution: false });
  });
for (const die of ['TU106', 'TU116'])
  test('die-family requirement selects a supported available die: ' + die, () => {
    const f = fixture();
    f.boxes = f.boxes.filter((b) => b.die === die);
    delete f.requirements.items[0].die;
    f.requirements.items[0].die_family = 'Turing';
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.die === die));
  });
for (const kind of ['proposed', 'unpinned'])
  test('non-executable trusted data never authorizes execution: ' + kind, () => {
    const f = fixture();
    if (kind === 'proposed') f.catalog.status = 'proposed';
    else {
      f.catalog.driver_pins[0].status = 'UNPINNED';
      f.catalog.driver_pins[0].sha256 = null;
      f.catalog.driver_pins[0].size = null;
    }
    const p = plan(f);
    assert.equal(p.admitted, false);
    rejects(() => validatePlan(f, p));
    validatePlan(f, p, { forExecution: false });
  });
for (const field of ['host', 'linux_guest'])
  test('unsupported/too-old driver does not cover requirement: ' + field, () => {
    const f = fixture();
    f.catalog.dies[0].minimum_driver_branches[field] = 590;
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.equal(p.assignments.length, 0);
  });
for (const field of ['machine_id', 'host_id'])
  test('bad identities excluded before selection: ' + field, () => {
    const f = fixture();
    f.limits[field === 'machine_id' ? 'bad_machine_ids' : 'bad_host_ids'] = [f.boxes[0][field]];
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a[field] !== f.boxes[0][field]));
  });
for (const field of ['inet_down_usd_gb', 'inet_up_usd_gb'])
  test('unit-rate caps reject expensive network: ' + field, () => {
    const f = fixture();
    f.boxes.forEach((b) => (b.pricing[field] = 0.021));
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.equal(p.assignments.length, 0);
  });
for (const value of [0.1, 0.5])
  test('total reservation capped before admission: ' + value, () => {
    const f = fixture();
    f.limits.cost_cap_usd = value;
    const p = plan(f);
    assert.equal(p.admitted, false);
    assert.ok(p.total_reserved_usd <= value + 1e-9);
  });
for (const mode of ['expensive', 'malformed'])
  test('invalid offer cannot hide behind a cheap compute rate: ' + mode, () => {
    const f = fixture();
    f.boxes[0].pricing.compute_usd_hour = mode === 'expensive' ? 100 : '0.01';
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.box_id !== 'offer-0'));
  });
for (const count of [2, 4])
  test('concurrency offsets reserve worst-case fallback intervals: ' + count, () => {
    const f = fixture();
    f.limits.max_concurrent_boxes = 1;
    f.requirements.items[0].runs = Array(count).fill('spec_lane_a');
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.length >= 2);
    const a = [...p.assignments].sort((a, b) => a.start_minute - b.start_minute);
    for (let i = 1; i < a.length; i++)
      assert.ok(a[i].start_minute >= a[i - 1].start_minute + a[i - 1].worst_case_minutes - 1e-8);
    validatePlan(f, p);
  });
for (const mutation of ['coverage', 'duplicate', 'minutes', 'cost', 'driver', 'order', 'machine'])
  for (const variant of [0, 1])
    test('validator refuses tampered ' + mutation + ' ' + variant, () => {
      const f = fixture();
      f.requirements.items[0].runs.push('spec_lane_a', 'spec_lane_b');
      const p = plan(f);
      const a = p.assignments[0];
      if (mutation === 'coverage') p.coverage.pop();
      if (mutation === 'duplicate') p.coverage.push(clone(p.coverage[variant]));
      if (mutation === 'minutes') a.estimated_minutes = variant ? 0 : 999;
      if (mutation === 'cost') a.pricing.reserved_usd = variant ? 0 : -1;
      if (mutation === 'driver') a.driver_pins[variant].sha256 = '0'.repeat(64);
      if (mutation === 'order') a.runs.reverse();
      if (mutation === 'machine') a.machine_id = 'foreign-' + variant;
      rejects(() => validatePlan(f, p));
    });
