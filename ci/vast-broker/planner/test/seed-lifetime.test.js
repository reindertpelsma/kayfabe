import test from 'node:test';
import assert from 'node:assert/strict';
import { plan, withoutSeeds, validatePlan } from '../index.js';
import { seeded } from './seed-fixture.js';
// FALSIFIER: direct seeding needs an exposed port, an expired server, or a mandatory fetch.
for (const n of [1, 2]) {
  test('no exposed port disables serving without losing coverage ' + n, () => {
    const f = seeded(n);
    for (const b of f.boxes) b.capabilities.can_expose_port = false;
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.seed_exports.length === 0 && a.seed_hints.length === 0));
    validatePlan(f, p);
  });
  test('one concurrent box cannot consume a direct seed after its source ends ' + n, () => {
    const f = seeded(n);
    f.limits.max_concurrent_boxes = 1;
    const p = plan(f);
    assert.equal(p.admitted, true);
    assert.ok(p.assignments.every((a) => a.seed_hints.length === 0));
  });
  test('direct seed fetch windows fit the charged server lifetime ' + n, () => {
    const f = seeded(n);
    f.limits.max_concurrent_boxes = n + 1;
    const p = plan(f);
    let checked = 0;
    for (const a of p.assignments)
      for (const h of a.seed_hints) {
        const source = p.assignments.find((x) => x.id === h.producer_assignment_id);
        assert.equal(h.fallback, 'build_local');
        assert.ok(h.fetch_until_minute <= source.seed_server.serve_until_minute);
        assert.ok(h.deadline_minute >= a.start_minute);
        assert.ok(
          source.seed_server.serve_until_minute <= source.start_minute + source.worst_case_minutes,
        );
        assert.ok(source.start_minute + source.estimated_minutes >= h.fetch_until_minute);
        checked++;
      }
    assert.ok(checked > 0);
    assert.ok(withoutSeeds(f, p).total_box_minutes >= p.total_box_minutes);
  });
  test('assignment has explicit optional wait instructions and complete cold fallback ' + n, () => {
    const f = seeded(n);
    f.limits.max_concurrent_boxes = n + 1;
    const p = plan(f),
      a = p.assignments.find((x) => x.seed_hints.length);
    assert.ok(a.execution_steps.some((s) => s.kind === 'wait_for_artifact'));
    for (const h of a.seed_hints) {
      const wait = a.execution_steps.find(
        (s) => s.kind === 'wait_for_artifact' && s.scope_key === h.scope_key,
      );
      assert.equal(wait.fallback, 'build_local');
      assert.equal(wait.deadline_minute, h.deadline_minute);
      assert.deepEqual(wait.fallback_step_ids, h.fallback_step_ids);
    }
    assert.deepEqual(
      a.execution_steps.filter((s) => s.kind === 'run').map((s) => s.step_id),
      a.runs.map((s) => s.id),
    );
  });
}
for (const n of [1, 2]) {
  test('direct serving minutes and cost include the final fetch window ' + n, () => {
    const f = seeded(1);
    f.requirements.items[0].runs = ['linux_image_build', 'kayfabe_build'];
    f.catalog.runs.find((r) => r.id === 'fixture_blocker').estimated_minutes = 43 + n;
    const p = plan(f),
      source = p.assignments.find((a) => a.seed_exports.length);
    assert.ok(source);
    assert.ok(source.estimated_minutes > source.fallback_minutes);
    assert.equal(
      source.start_minute + source.estimated_minutes,
      source.seed_server.serve_until_minute,
    );
    assert.ok(
      source.pricing.estimated_usd + 1e-6 >=
        (source.pricing.hourly_usd * source.estimated_minutes) / 60 + source.pricing.transfer_usd,
    );
  });
  test('direct seed port is a data value ' + n, () => {
    const f = seeded(1);
    f.limits.seeding.container_port = 9000 + n;
    const source = plan(f).assignments.find((a) => a.seed_exports.length);
    assert.equal(source.seed_server.container_port, 9000 + n);
  });
}
