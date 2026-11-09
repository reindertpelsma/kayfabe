import test from 'node:test';
import assert from 'node:assert/strict';
import { plan } from '../index.js';
import { market, oracle, execute } from './simulation-model.js';
import { fixture, clone } from './helpers.js';
import { seeded } from './seed-fixture.js';
const count = Number(process.env.KF_PLANNER_SEEDS ?? 200),
  first = Number(process.env.KF_PLANNER_START ?? 1337);
assert.ok(
  Number.isSafeInteger(count) &&
    count >= 1 &&
    count <= 2000 &&
    Number.isSafeInteger(first) &&
    first >= 0 &&
    first + count <= 2 ** 32,
  'bounded fixed seeds',
);
// FALSIFIER: any deterministic market admits an over-cap/duplicate/unsafe rental,
// loses a requirement, depends on input object order, or leaks a provider instance.
for (let i = 0; i < count; i++)
  test('planner market seed ' + (first + i), async () => {
    const f = market(first + i),
      before = JSON.stringify(f),
      p = plan(f);
    assert.equal(JSON.stringify(f), before);
    oracle(f, p);
    const g = clone(f);
    for (const k of ['runs', 'artifacts', 'dies', 'driver_pins', 'driver_sets', 'profiles'])
      g.catalog[k].reverse();
    g.requirements.items.reverse();
    g.boxes.reverse();
    g.boxes = g.boxes.map((b) => Object.fromEntries(Object.entries(b).reverse()));
    g.limits.price_caps.reverse();
    assert.equal(
      JSON.stringify(plan(g)),
      JSON.stringify(p),
      'same semantic input has canonical output bytes',
    );
    const result = await execute(f, p);
    if (p.admitted) assert.equal(result.cloud.creates, p.assignments.length);
    else assert.equal(result.cloud.creates, 0);
  });
for (const fault of [
  'offline-once',
  'api-500',
  'api-429',
  'api-timeout',
  'ambiguous-create',
  'offer-disappears',
  'third-party',
  'restart',
  'partial-five',
])
  test('assignment simulation worst: ' + fault, async () => {
    const f = market(1338);
    f.limits.max_concurrent_boxes = 8;
    const p = plan(f);
    assert.equal(p.admitted, true);
    oracle(f, p);
    const r = await execute(f, p, { fault });
    if (fault === 'ambiguous-create') {
      assert.equal(r.cloud.creates, p.assignments.length);
      assert.ok(r.records.some((r) => r.idSource === 'reconcile'));
    }
    if (fault === 'partial-five') assert.equal(r.cloud.creates, 5);
    if (fault === 'offer-disappears') assert.equal(r.cloud.creates, 0);
    assert.ok(r.records.every((r) => r.ended !== null));
  });
for (const provider of ['workstation', 'container'])
  test('non-Vast fixed-host self-run planning ' + provider, () => {
    const f = fixture();
    f.boxes = f.boxes.slice(0, 1);
    f.boxes[0].provider = provider;
    f.boxes[0].capabilities.can_swap_host_driver = false;
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.equal(p.assignments[0].provider, provider);
    oracle(f, p);
  });
test('assignment simulation worst: seeder dies during consumer download; local fallback remains valid', async () => {
  const f = seeded(1);
  f.limits.max_concurrent_boxes = 2;
  const d = { ...clone(f.catalog.dies[1]), id: 'TU104' };
  f.catalog.dies.push(d);
  f.catalog.driver_sets[0].supported_dies.push(d.id);
  const q = {
    ...clone(f.requirements.items[0]),
    id: 'fast-primary',
    die: d.id,
    runs: ['windows_gate'],
  };
  f.requirements.items.push(q);
  const b = {
    ...clone(f.boxes[2]),
    id: 'fast-box',
    machine_id: 'fast-machine',
    host_id: 'fast-host',
    die: d.id,
  };
  f.boxes.push(b);
  const p = plan(f);
  assert.equal(p.admitted, true);
  assert.ok(p.assignments.some((a) => a.seed_hints.length));
  oracle(f, p);
  const result = await execute(f, p, { fault: 'seeder-dies' });
  assert.equal(result.fallbacks, 1);
});
