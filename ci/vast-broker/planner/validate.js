import { assert, checkSchema, same } from './schema.js';
import { prepare, executableData, buildSteps, profileFor, eligible, withinPrice } from './model.js';
import { materialize, assemble, maximumConcurrency } from './plan.js';
function validateExecutionOrder(ctx, a) {
  const available = new Set(),
    ids = new Set(),
    swapped = new Set();
  let activeHost = null;
  const context = (s) =>
    JSON.stringify([s.drivers.host, s.drivers.linux_guest, s.drivers.windows_guest]);
  const swapContexts = new Set(
    a.runs.filter((s) => ctx.runs.get(s.run_id)?.host_swap).map(context),
  );
  for (const step of a.runs) {
    const run = ctx.runs.get(step.run_id);
    assert(run && !ids.has(step.id), 'RUN_ORDER');
    ids.add(step.id);
    if (activeHost !== null && activeHost !== step.drivers.host)
      assert(run.host_swap && !a.primary_evidence, 'HOST_TRANSITION');
    if (run.host_swap)
      assert(!a.primary_evidence && a.capabilities.can_swap_host_driver, 'HOST_SWAP');
    if (run.host_swap) swapped.add(context(step));
    else if (run.driver_axes.length && swapContexts.has(context(step)))
      assert(swapped.has(context(step)), 'HOST_TRANSITION');
    activeHost = step.drivers.host;
    const key = (id) =>
      id +
      '@' +
      (ctx.artifacts.get(id).scope === 'driver'
        ? JSON.stringify([step.drivers.host, step.drivers.linux_guest, step.drivers.windows_guest])
        : 'box');
    assert(
      same(step.needs, [...run.needs].sort().map(key)) &&
        same(step.produces, [...run.produces].sort().map(key)),
      'ARTIFACT_SCOPE',
    );
    for (const need of step.needs) assert(available.has(need), 'RUN_ORDER');
    for (const produced of step.produces) {
      assert(!available.has(produced), 'SETUP_DUPLICATE');
      available.add(produced);
    }
    if (step.optional_seed_for !== null)
      assert(
        step.coverage.length === 0 &&
          step.produces.length === 1 &&
          step.produces[0] === step.optional_seed_for,
        'SEED_FALLBACK',
      );
  }
}
export function validatePlan(input, p, options = {}) {
  return validatePrepared(prepare(input), p, options);
}
export function validatePrepared(ctx, p, { forExecution = true } = {}) {
  checkSchema('plan', p);
  const tokenMap = new Map(ctx.tokens.map((t) => [t.key, t])),
    seen = new Set(),
    assignmentIds = new Set(),
    machines = new Set();
  assert(p.coverage.length === ctx.tokens.length, 'COVERAGE');
  for (const c of p.coverage) {
    assert(tokenMap.has(c.key) && !seen.has(c.key), 'COVERAGE');
    seen.add(c.key);
  }
  assert(p.assignments.length <= ctx.limits.max_boxes, 'BOX_LIMIT');
  const bins = [];
  for (const a of p.assignments) {
    validateExecutionOrder(ctx, a);
    assert(!assignmentIds.has(a.id), 'ASSIGNMENT_DUPLICATE');
    assignmentIds.add(a.id);
    const box = ctx.boxMap.get(a.box_id);
    assert(box, 'UNKNOWN_BOX');
    assert(!machines.has(box.machine_id), 'MACHINE_DUPLICATE');
    machines.add(box.machine_id);
    const rows = p.coverage.filter((c) => c.status === 'covered' && c.assignment_id === a.id),
      tokens = rows.map((c) => tokenMap.get(c.key));
    assert(tokens.length > 0, 'COVERAGE');
    const built = buildSteps(ctx, tokens),
      profile = profileFor(ctx, tokens);
    assert(eligible(ctx, box, tokens, profile, built), 'INELIGIBLE_BOX');
    bins.push({ id: a.id, box, tokens });
  }
  for (const c of p.coverage)
    assert(c.status !== 'covered' || assignmentIds.has(c.assignment_id), 'COVERAGE');
  const edges = p.assignments.flatMap((a) =>
    a.seed_hints.map((h) => ({
      source: h.producer_assignment_id,
      target: a.id,
      scope_key: h.scope_key,
    })),
  );
  const assignments = materialize(ctx, bins, edges);
  const expected = assemble(
    ctx,
    bins,
    assignments,
    new Map(
      p.coverage
        .filter((c) => c.status !== 'covered')
        .map((c) => [c.key, 'NO_CAPACITY_WITHIN_CAPS']),
    ),
  );
  // No supplied proof, minute, price, source, driver or deadline is authoritative.
  assert(
    same(p, expected),
    'PLAN_MISMATCH',
    'Plan differs from trusted requirements, catalog, box facts or limits',
  );
  assert(p.total_reserved_usd <= ctx.limits.cost_cap_usd, 'COST_CAP');
  assert(
    p.assignments.every((a) => withinPrice(ctx, ctx.profiles.get(a.tier), a.pricing)),
    'PRICE_CAP',
  );
  assert(
    maximumConcurrency(
      p.assignments.map((a) => ({
        start: a.start_minute,
        end: a.start_minute + a.worst_case_minutes,
      })),
    ) <= ctx.limits.max_concurrent_boxes,
    'CONCURRENCY',
  );
  const execution = executableData(ctx, assignments) && p.admitted;
  if (forExecution) assert(execution, 'NOT_EXECUTABLE');
  return { valid: true, executable: execution };
}
export function validateAssignment(input, a, { plan: parent, forExecution = true } = {}) {
  checkSchema('assignment', a);
  if (parent) {
    const result = validatePlan(input, parent, { forExecution });
    assert(
      parent.assignments.some((x) => x.id === a.id && same(x, a)),
      'ASSIGNMENT_MISMATCH',
    );
    return result;
  }
  const ctx = prepare(input);
  assert(
    a.seed_hints.length === 0,
    'SEED_CONTEXT',
    'Seeded assignments require the full validated plan',
  );
  const box = ctx.boxMap.get(a.box_id);
  assert(box, 'UNKNOWN_BOX');
  const keys = new Set(a.runs.flatMap((s) => s.coverage));
  assert(keys.size === ctx.tokens.length && ctx.tokens.every((t) => keys.has(t.key)), 'COVERAGE');
  const bins = [{ id: a.id, box, tokens: ctx.tokens }],
    expected = materialize(ctx, bins, [])[0];
  assert(same(a, expected), 'ASSIGNMENT_MISMATCH');
  const p = assemble(ctx, bins, [expected]);
  return validatePrepared(ctx, p, { forExecution });
}
export function validateInput(input, { forExecution = false } = {}) {
  const ctx = prepare(input),
    ready = executableData(ctx);
  if (forExecution) assert(ready, 'NOT_EXECUTABLE');
  return { valid: true, executable: ready };
}
