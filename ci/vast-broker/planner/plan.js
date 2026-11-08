import { assert, canonical, compare, PlanError } from './schema.js';
import {
  prepare,
  buildSteps,
  profileFor,
  matchesTarget,
  eligible,
  quote,
  withinPrice,
  round,
  executableData,
} from './model.js';
const lex = compare;
const sum = (xs, key) => round(xs.reduce((n, x) => n + x[key], 0));
export function maximumConcurrency(intervals) {
  const events = intervals
    .flatMap(({ start, end }) =>
      end > start
        ? [
            [start, 1],
            [end, -1],
          ]
        : [],
    )
    .sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  let n = 0,
    max = 0;
  for (const [, delta] of events) {
    n += delta;
    max = Math.max(max, n);
  }
  return max;
}
function nextSlot(earliest, minutes, intervals, cap) {
  for (const start of [
    ...new Set([earliest, ...intervals.map((x) => x.end).filter((t) => t >= earliest)]),
  ].sort((a, b) => a - b))
    if (maximumConcurrency([...intervals, { start, end: round(start + minutes) }]) <= cap)
      return start;
  assert(false, 'SCHEDULE');
}
function baseAssignment(ctx, bin) {
  const { box, tokens, id } = bin,
    built = buildSteps(ctx, tokens),
    profile = profileFor(ctx, tokens);
  assert(eligible(ctx, box, tokens, profile, built), 'INELIGIBLE_BOX');
  const pins = new Set();
  for (const s of built.steps) for (const id of Object.values(s.drivers)) pins.add(id);
  return {
    version: 1,
    status: 'proposed',
    id,
    box_id: box.id,
    machine_id: box.machine_id,
    host_id: box.host_id,
    die: box.die,
    die_family: ctx.dies.get(box.die).die_family,
    provider: box.provider,
    tier: profile.tier,
    capabilities: { ...box.capabilities },
    source_revision: ctx.input.source_revision,
    catalog_revision: ctx.catalog.revision,
    primary_evidence: tokens.some((t) => t.evidence === 'primary'),
    start_minute: 0,
    estimated_minutes: built.minutes,
    fallback_minutes: built.minutes,
    worst_case_minutes: built.minutes,
    reservation_minutes: Math.min(ctx.limits.hard_minutes, box.limits.hard_minutes),
    runs: built.steps.map((s) => ({ ...s })),
    setup_reuse: built.reuse,
    seed_hints: [],
    seed_exports: [],
    seed_server: null,
    execution_steps: [],
    driver_pins: [...pins].sort().map((id) => ({ ...ctx.pins.get(id) })),
    artifact_sources: built.reuse.map((r) => ({
      artifact: r.artifact,
      scope_key: r.scope_key,
      tier: profile.tier,
      kind: 'local',
      producer_assignment_id: null,
    })),
    assigned_consumers: [],
    pricing: quote(ctx, box, profile, built),
  };
}
const transferMinutes = (ctx, box, artifact, direction) =>
  artifact.gb === 0
    ? 0
    : round(
        (artifact.gb * 8000) /
          (box.limits[direction === 'up' ? 'inet_up_mbps' : 'inet_down_mbps'] * 60),
      );
function exportSchedule(ctx, bin, assignment, edges) {
  const result = [];
  let cursor = 0;
  for (const e of [...edges].sort(
    (a, b) => lex(a.target, b.target) || lex(a.scope_key, b.scope_key),
  )) {
    const artifact = ctx.artifacts.get(e.scope_key.split('@')[0]);
    assert(artifact, 'SEED_PRODUCER');
    let ready = 0,
      producer = null;
    for (const s of assignment.runs) {
      ready += s.minutes;
      if (s.produces.includes(e.scope_key)) {
        producer = s;
        break;
      }
    }
    assert(producer, 'SEED_PRODUCER');
    const minutes = transferMinutes(ctx, bin.box, artifact, 'up');
    assert(
      Number.isFinite(minutes) && minutes <= ctx.limits.seeding.max_wait_minutes,
      'SEED_DEADLINE',
    );
    const start = Math.max(cursor, ready);
    cursor = round(start + minutes);
    result.push({
      artifact: artifact.id,
      scope_key: e.scope_key,
      consumer_assignment_id: e.target,
      ready_after_step_id: producer.id,
      gb: artifact.gb,
      minutes,
      available_minute: cursor,
      deadline_minute: round(start + ctx.limits.seeding.max_wait_minutes),
    });
  }
  return result;
}
// Materialization evaluates an explicit seed graph. It neither selects offers nor
// invents graph edges, so the independent validator can use exactly this accounting.
export function materialize(ctx, bins, edges = []) {
  const coldById = edges.length ? new Map(materialize(ctx, bins, []).map((a) => [a.id, a])) : null;
  const byBin = new Map(bins.map((b) => [b.id, b])),
    assignments = new Map(bins.map((b) => [b.id, baseAssignment(ctx, b)]));
  const incoming = new Map(),
    outgoing = new Map(),
    keys = new Set();
  for (const e of edges) {
    assert(byBin.has(e.source) && byBin.has(e.target) && e.source !== e.target, 'SEED_EDGE');
    const key = e.target + ':' + e.scope_key;
    assert(!keys.has(key), 'SEED_DUPLICATE');
    keys.add(key);
    if (!incoming.has(e.target)) incoming.set(e.target, []);
    incoming.get(e.target).push(e);
    if (!outgoing.has(e.source)) outgoing.set(e.source, []);
    outgoing.get(e.source).push(e);
  }
  for (const source of outgoing.keys())
    assert(!incoming.has(source), 'SEED_CYCLE', 'v1 seed sources must build locally');
  const order = [...assignments.keys()].sort(
    (a, b) =>
      Number(incoming.has(a)) - Number(incoming.has(b)) ||
      Number(!outgoing.has(a)) - Number(!outgoing.has(b)) ||
      lex(a, b),
  );
  const intervals = [];
  for (const id of order) {
    const a = assignments.get(id),
      bin = byBin.get(id),
      profile = profileFor(ctx, bin.tokens),
      built = buildSteps(ctx, bin.tokens),
      hints = [];
    const inEdges = [...(incoming.get(id) ?? [])].sort((a, b) => lex(a.scope_key, b.scope_key));
    let earliest = 0,
      skipped = 0,
      transfer = 0;
    for (const e of inEdges) {
      const producer = assignments.get(e.source),
        source = byBin.get(e.source),
        artifact = ctx.artifacts.get(e.scope_key.split('@')[0]);
      assert(ctx.limits.seeding.enabled && artifact?.seedable, 'SEED_POLICY');
      assert(
        artifact.reuse_policy === 'trusted_content' && artifact.trusted_content !== null,
        'SEED_TRUST',
      );
      assert(profileFor(ctx, source.tokens).can_seed, 'SEED_POLICY');
      assert(source.box.capabilities.can_expose_port === true, 'SEED_PORT');
      assert(producer.die_family === a.die_family, 'SEED_DIE_FAMILY');
      assert(
        profile.artifact_sources.some(
          (p) => p.artifact === artifact.id && p.policy === 'optional_seed',
        ),
        'SEED_POLICY',
      );
      const local = a.runs.find((s) => s.produces.includes(e.scope_key)),
        sourceStep = producer.runs.find((s) => s.produces.includes(e.scope_key));
      assert(
        local &&
          sourceStep &&
          ctx.runs.get(local.run_id).setup &&
          local.produces.length === 1 &&
          local.coverage.length === 0,
        'SEED_FALLBACK',
      );
      const exported = producer.seed_exports.find(
        (x) => x.consumer_assignment_id === id && x.scope_key === e.scope_key,
      );
      assert(exported, 'SEED_PRODUCER');
      const available = exported.available_minute,
        minutes = transferMinutes(ctx, bin.box, artifact, 'down');
      assert(
        Number.isFinite(minutes) && minutes <= ctx.limits.seeding.max_wait_minutes + 1e-7,
        'SEED_DEADLINE',
      );
      const wait = Math.max(0, available - (coldById?.get(id)?.start_minute ?? 0));
      assert(
        local.minutes - minutes - exported.minutes - wait >
          ctx.limits.seeding.min_savings_minutes + 1e-7,
        'SEED_SAVINGS',
      );
      earliest = Math.max(earliest, available);
      skipped += local.minutes;
      transfer += minutes;
      local.optional_seed_for = e.scope_key;
      hints.push({
        artifact: artifact.id,
        scope_key: e.scope_key,
        producer_assignment_id: e.source,
        producer_box_id: producer.box_id,
        available_minute: available,
        transfer_minutes: minutes,
        deadline_minute: 0,
        fetch_until_minute: 0,
        fallback: 'build_local',
        trusted_content: artifact.trusted_content,
        fallback_step_ids: [local.id],
        gb: artifact.gb,
      });
    }
    a.seed_hints = hints;
    const exports = [...(outgoing.get(id) ?? [])].sort(
      (a, b) => lex(a.target, b.target) || lex(a.scope_key, b.scope_key),
    );
    a.seed_exports = exportSchedule(ctx, bin, a, exports);
    const exportTail = Math.max(
      0,
      ...a.seed_exports.map((e) => e.available_minute - a.fallback_minutes),
    );
    a.estimated_minutes = round(a.fallback_minutes - skipped + transfer + exportTail);
    a.worst_case_minutes = round(
      a.fallback_minutes +
        (hints.length ? ctx.limits.seeding.max_wait_minutes : 0) +
        exports.length * ctx.limits.seeding.max_wait_minutes,
    );
    assert(
      a.worst_case_minutes <= a.reservation_minutes - ctx.limits.margin_minutes + 1e-7,
      'FALLBACK_TIME',
    );
    a.start_minute = nextSlot(
      earliest,
      a.worst_case_minutes,
      intervals,
      ctx.limits.max_concurrent_boxes,
    );
    for (const h of hints) {
      const source = assignments.get(h.producer_assignment_id);
      const end = round(source.start_minute + source.worst_case_minutes);
      h.deadline_minute = Math.min(
        round(a.start_minute + ctx.limits.seeding.max_wait_minutes),
        round(end - h.transfer_minutes),
      );
      h.fetch_until_minute = round(h.deadline_minute + h.transfer_minutes);
      assert(
        h.deadline_minute >= a.start_minute && h.fetch_until_minute <= end + 1e-7,
        'SEED_LIFETIME',
      );
    }
    for (const e of a.seed_exports) {
      e.available_minute = round(e.available_minute + a.start_minute);
      e.deadline_minute = round(e.deadline_minute + a.start_minute);
    }
    intervals.push({ start: a.start_minute, end: round(a.start_minute + a.worst_case_minutes) });
    const consumers = [...(outgoing.get(id) ?? [])];
    a.assigned_consumers = [...new Set(consumers.map((e) => e.target))].sort();
    const upload =
      profile.base_upload_gb +
      consumers.reduce((n, e) => n + ctx.artifacts.get(e.scope_key.split('@')[0]).gb, 0);
    a.pricing = quote(ctx, bin.box, profile, built, {
      uploadGb: upload,
      minutes: a.estimated_minutes,
      downloadExtraGb: hints.reduce((n, h) => n + h.gb, 0),
    });
    for (const source of a.artifact_sources) {
      const h = hints.find((h) => h.scope_key === source.scope_key);
      if (h) {
        source.kind = 'optional_seed';
        source.producer_assignment_id = h.producer_assignment_id;
      } else if (
        profile.artifact_sources.some(
          (p) => p.artifact === source.artifact && p.policy === 'official',
        )
      )
        source.kind = 'official';
    }
  }
  const out = [...assignments.values()].sort((a, b) => lex(a.id, b.id));
  for (const source of out.filter((a) => a.seed_exports.length)) {
    const hints = out
      .flatMap((a) => a.seed_hints)
      .filter((h) => h.producer_assignment_id === source.id);
    const end = round(
      Math.max(
        source.start_minute + source.estimated_minutes,
        ...hints.map((h) => h.fetch_until_minute),
      ),
    );
    assert(end <= source.start_minute + source.worst_case_minutes + 1e-7, 'SEED_LIFETIME');
    source.seed_server = {
      container_port: ctx.limits.seeding.container_port,
      serve_until_minute: end,
    };
    source.estimated_minutes = round(end - source.start_minute);
    const bin = byBin.get(source.id);
    source.pricing = quote(ctx, bin.box, profileFor(ctx, bin.tokens), buildSteps(ctx, bin.tokens), {
      uploadGb: source.pricing.bandwidth_up_gb,
      minutes: source.estimated_minutes,
    });
  }
  for (const a of out)
    a.execution_steps = [
      ...a.seed_hints.map((h) => ({
        kind: 'wait_for_artifact',
        artifact: h.artifact,
        scope_key: h.scope_key,
        deadline_minute: h.deadline_minute,
        fetch_until_minute: h.fetch_until_minute,
        minutes: round(h.deadline_minute - a.start_minute),
        fallback: 'build_local',
        fallback_step_ids: [...h.fallback_step_ids],
      })),
      ...a.runs.map((s) => ({ kind: 'run', step_id: s.id })),
    ];
  const execution = executableData(ctx, out);
  for (const a of out) a.status = execution ? 'executable' : 'proposed';
  return out;
}
function chooseSeeds(ctx, bins) {
  if (!ctx.limits.seeding.enabled) return [];
  const cold = materialize(ctx, bins),
    byId = new Map(bins.map((b) => [b.id, b])),
    edges = [];
  for (const target of cold) {
    const targetBin = byId.get(target.id),
      profile = profileFor(ctx, targetBin.tokens);
    if (
      profile.can_seed ||
      target.fallback_minutes + ctx.limits.seeding.max_wait_minutes >
        target.reservation_minutes - ctx.limits.margin_minutes
    )
      continue;
    for (const step of [...target.runs].sort((a, b) =>
      lex(a.produces[0] ?? '', b.produces[0] ?? ''),
    )) {
      if (step.produces.length !== 1 || step.coverage.length || !ctx.runs.get(step.run_id).setup)
        continue;
      const key = step.produces[0],
        artifact = ctx.artifacts.get(key.split('@')[0]);
      if (
        !artifact.seedable ||
        artifact.reuse_policy !== 'trusted_content' ||
        !profile.artifact_sources.some(
          (p) => p.artifact === artifact.id && p.policy === 'optional_seed',
        )
      )
        continue;
      const sources = cold.filter(
        (s) =>
          s.id !== target.id &&
          s.die_family === target.die_family &&
          profileFor(ctx, byId.get(s.id).tokens).can_seed &&
          byId.get(s.id).box.capabilities.can_expose_port === true &&
          s.runs.some((r) => r.produces.includes(key)),
      );
      const choices = sources
        .map((source) => {
          const sourceBox = byId.get(source.id).box,
            up = transferMinutes(ctx, sourceBox, artifact, 'up'),
            minutes = transferMinutes(ctx, targetBin.box, artifact, 'down');
          if (
            !Number.isFinite(up) ||
            !Number.isFinite(minutes) ||
            up > ctx.limits.seeding.max_wait_minutes ||
            minutes > ctx.limits.seeding.max_wait_minutes
          )
            return { source, key, minutes, up, fits: false, savings: -Infinity };
          const prior = edges.filter((e) => e.source === source.id),
            exports = exportSchedule(ctx, byId.get(source.id), source, [
              ...prior,
              { source: source.id, target: target.id, scope_key: key },
            ]),
            published = exports.find(
              (e) => e.consumer_assignment_id === target.id && e.scope_key === key,
            ),
            wait = Math.max(
              0,
              source.start_minute + published.available_minute - target.start_minute,
            ),
            fits =
              source.fallback_minutes + (prior.length + 1) * ctx.limits.seeding.max_wait_minutes <=
              source.reservation_minutes - ctx.limits.margin_minutes;
          return { source, key, minutes, up, fits, savings: step.minutes - minutes - up - wait };
        })
        .filter(
          (c) =>
            c.fits &&
            c.minutes <= ctx.limits.seeding.max_wait_minutes &&
            c.up <= ctx.limits.seeding.max_wait_minutes &&
            c.savings > ctx.limits.seeding.min_savings_minutes,
        )
        .sort((a, b) => b.savings - a.savings || lex(a.source.id, b.source.id));
      if (choices[0])
        edges.push({ source: choices[0].source.id, target: target.id, scope_key: key });
    }
  }
  return edges;
}
const feasiblePrices = (ctx, assignments) =>
  assignments.every((a) => withinPrice(ctx, ctx.profiles.get(a.tier), a.pricing)) &&
  sum(
    assignments.map((a) => a.pricing),
    'reserved_usd',
  ) <= ctx.limits.cost_cap_usd;
function seededAssignments(ctx, bins) {
  let edges = chooseSeeds(ctx, bins),
    out;
  const cold = materialize(ctx, bins, []);
  for (;;) {
    try {
      out = materialize(ctx, bins, edges);
      break;
    } catch (e) {
      if (!edges.length || !(e instanceof PlanError) || e.code !== 'SEED_LIFETIME') throw e;
      edges.pop();
    }
  }
  // Seeding is optional. Never make an otherwise capped cold plan unsafe.
  while (
    edges.length &&
    (!feasiblePrices(ctx, out) || sum(out, 'estimated_minutes') > sum(cold, 'estimated_minutes'))
  ) {
    edges.pop();
    try {
      out = materialize(ctx, bins, edges);
    } catch (e) {
      if (!(e instanceof PlanError) || e.code !== 'SEED_LIFETIME') throw e;
      return { assignments: cold, edges: [] };
    }
  }
  return { assignments: out, edges };
}
function seedScore(ctx, assignment, box) {
  const s = ctx.limits.seeding,
    p = assignment.pricing;
  return (
    p.effective_usd_hour * s.price_weight +
    box.pricing.inet_up_usd_gb * s.upload_cost_weight -
    (assignment.assigned_consumers.length
      ? Math.log1p(box.limits.inet_up_mbps) * s.upload_speed_weight
      : 0)
  );
}
function repriceSeeders(ctx, bins) {
  let current = seededAssignments(ctx, bins);
  for (const bin of bins) {
    const a = current.assignments.find((a) => a.id === bin.id);
    if (!a.assigned_consumers.length) continue;
    const used = new Set(bins.filter((b) => b.id !== bin.id).map((b) => b.box.machine_id));
    const built = buildSteps(ctx, bin.tokens),
      profile = profileFor(ctx, bin.tokens);
    let best = { box: bin.box, state: current, score: seedScore(ctx, a, bin.box) };
    for (const box of ctx.boxes) {
      if (
        box.die !== bin.box.die ||
        used.has(box.machine_id) ||
        !eligible(ctx, box, bin.tokens, profile, built)
      )
        continue;
      const trial = bins.map((b) => (b.id === bin.id ? { ...b, box } : b));
      let state;
      try {
        state = seededAssignments(ctx, trial);
      } catch (e) {
        if (
          e instanceof PlanError &&
          ['SEED_DEADLINE', 'SEED_SAVINGS', 'FALLBACK_TIME'].includes(e.code)
        )
          continue;
        throw e;
      }
      if (!feasiblePrices(ctx, state.assignments)) continue;
      const candidate = state.assignments.find((a) => a.id === bin.id);
      // A role-aware replacement must still serve the assigned consumers.
      if (!a.assigned_consumers.every((id) => candidate.assigned_consumers.includes(id))) continue;
      const score = seedScore(ctx, candidate, box);
      if (
        score < best.score - 1e-9 ||
        (Math.abs(score - best.score) <= 1e-9 && lex(box.id, best.box.id) < 0)
      )
        best = { box, state, score };
    }
    bin.box = best.box;
    current = best.state;
  }
  return current.assignments;
}
export function assemble(ctx, bins, assignments, uncovered = new Map()) {
  const coverage = ctx.tokens.map((t) => {
    for (const a of assignments) {
      const step = a.runs.find((s) => s.coverage.includes(t.key));
      if (step)
        return {
          key: t.key,
          requirement_id: t.requirement_id,
          run_index: t.run_index,
          driver_set_id: t.driver_set_id,
          run_id: t.run_id,
          status: 'covered',
          die: a.die,
          assignment_id: a.id,
          step_id: step.id,
          reason: null,
        };
    }
    const targetOffered = ctx.boxes.some((b) =>
      t.target.die
        ? b.die === t.target.die
        : ctx.dies.get(b.die)?.die_family === t.target.die_family,
    );
    return {
      key: t.key,
      requirement_id: t.requirement_id,
      run_index: t.run_index,
      driver_set_id: t.driver_set_id,
      run_id: t.run_id,
      status: targetOffered ? 'unsatisfied' : 'not_offered',
      die: t.target.die ?? null,
      assignment_id: null,
      step_id: null,
      reason: targetOffered ? (uncovered.get(t.key) ?? 'NO_CAPACITY_WITHIN_CAPS') : 'NOT_OFFERED',
    };
  });
  const execution = executableData(ctx, assignments),
    refusals = [];
  if (!coverage.every((c) => c.status === 'covered')) refusals.push('INCOMPLETE');
  if (!execution) refusals.push('DATA_NOT_EXECUTABLE');
  const reserved = sum(
    assignments.map((a) => a.pricing),
    'reserved_usd',
  );
  if (reserved > ctx.limits.cost_cap_usd) refusals.push('COST_CAP');
  if (!assignments.every((a) => withinPrice(ctx, ctx.profiles.get(a.tier), a.pricing)))
    refusals.push('PRICE_CAP');
  const tiers = [...new Set(assignments.map((a) => a.tier))].sort().map((tier) => {
    const rows = assignments.filter((a) => a.tier === tier);
    return {
      tier,
      boxes: rows.length,
      estimated_usd: sum(
        rows.map((a) => a.pricing),
        'estimated_usd',
      ),
      reserved_usd: sum(
        rows.map((a) => a.pricing),
        'reserved_usd',
      ),
    };
  });
  return {
    version: 1,
    status: execution ? 'executable' : 'proposed',
    source_revision: ctx.input.source_revision,
    catalog_revision: ctx.catalog.revision,
    assignments,
    coverage,
    rejected_candidates: ctx.rejected,
    total_box_minutes: sum(assignments, 'estimated_minutes'),
    finish_minute: round(
      Math.max(0, ...assignments.map((a) => a.start_minute + a.estimated_minutes)),
    ),
    reserved_finish_minute: round(
      Math.max(0, ...assignments.map((a) => a.start_minute + a.worst_case_minutes)),
    ),
    total_estimated_usd: sum(
      assignments.map((a) => a.pricing),
      'estimated_usd',
    ),
    total_reserved_usd: reserved,
    tier_totals: tiers,
    refusals,
    admitted: refusals.length === 0,
  };
}
export function planPrepared(ctx) {
  const bins = [],
    uncovered = new Map();
  const singleMinutes = new Map(ctx.tokens.map((t) => [t.key, buildSteps(ctx, [t]).minutes]));
  const tokens = [...ctx.tokens].sort(
    (a, b) =>
      Number(a.evidence !== 'primary') - Number(b.evidence !== 'primary') ||
      Number(!ctx.profiles.get(a.tier).can_seed) - Number(!ctx.profiles.get(b.tier).can_seed) ||
      singleMinutes.get(b.key) - singleMinutes.get(a.key) ||
      lex(a.key, b.key),
  );
  for (const t of tokens) {
    const used = new Set(bins.map((b) => b.box.machine_id)),
      choices = [];
    for (const bin of bins) {
      const ts = [...bin.tokens, t],
        built = buildSteps(ctx, ts),
        profile = profileFor(ctx, ts);
      if (!eligible(ctx, bin.box, ts, profile, built)) continue;
      const price = quote(ctx, bin.box, profile, built),
        old = quote(ctx, bin.box, profileFor(ctx, bin.tokens), buildSteps(ctx, bin.tokens));
      const total =
        bins.reduce(
          (sum, b) =>
            sum +
            quote(ctx, b.box, profileFor(ctx, b.tokens), buildSteps(ctx, b.tokens)).reserved_usd,
          0,
        ) +
        price.reserved_usd -
        old.reserved_usd;
      if (withinPrice(ctx, profile, price) && total <= ctx.limits.cost_cap_usd)
        choices.push({
          bin,
          built,
          profile,
          price,
          increment: price.reserved_usd - old.reserved_usd,
          box: bin.box,
        });
    }
    if (bins.length < ctx.limits.max_boxes) {
      const built = buildSteps(ctx, [t]),
        profile = profileFor(ctx, [t]),
        reserved = bins.reduce(
          (sum, b) =>
            sum +
            quote(ctx, b.box, profileFor(ctx, b.tokens), buildSteps(ctx, b.tokens)).reserved_usd,
          0,
        );
      for (const box of ctx.boxes) {
        if (used.has(box.machine_id) || !eligible(ctx, box, [t], profile, built)) continue;
        const price = quote(ctx, box, profile, built);
        if (
          withinPrice(ctx, profile, price) &&
          reserved + price.reserved_usd <= ctx.limits.cost_cap_usd
        )
          choices.push({ bin: null, built, profile, price, increment: price.reserved_usd, box });
      }
    }
    choices.sort(
      (a, b) =>
        a.increment - b.increment ||
        Number(!a.bin) - Number(!b.bin) ||
        a.price.effective_usd_hour - b.price.effective_usd_hour ||
        lex(a.box.id, b.box.id),
    );
    const best = choices[0];
    if (!best) {
      uncovered.set(t.key, 'NO_CAPACITY_WITHIN_CAPS');
      continue;
    }
    if (best.bin) best.bin.tokens.push(t);
    else
      bins.push({
        id: 'box-' + String(bins.length + 1).padStart(3, '0'),
        box: best.box,
        tokens: [t],
      });
  }
  return assemble(ctx, bins, repriceSeeders(ctx, bins), uncovered);
}
export function withoutSeedsPrepared(ctx, p) {
  const byToken = new Map(ctx.tokens.map((t) => [t.key, t]));
  const bins = p.assignments.map((a) => ({
    id: a.id,
    box: ctx.boxMap.get(a.box_id),
    tokens: p.coverage.filter((c) => c.assignment_id === a.id).map((c) => byToken.get(c.key)),
  }));
  return assemble(
    ctx,
    bins,
    materialize(ctx, bins, []),
    new Map(p.coverage.filter((c) => c.status !== 'covered').map((c) => [c.key, c.reason])),
  );
}
