import { assert, checkSchema, canonical, same, PlanError, compare } from './schema.js';
export const round = (x) => Math.ceil((x - 1e-12) * 1e6) / 1e6;
export const sorted = (xs) => [...xs].sort((a, b) => compare(a.id, b.id));
export const driversKey = (d) => canonical([d.host, d.linux_guest, d.windows_guest]);
const unique = (xs, key = (x) => x.id) => {
  const m = new Map();
  for (const x of xs) {
    const k = key(x);
    assert(!m.has(k), 'DUPLICATE', String(k));
    m.set(k, x);
  }
  return m;
};
export function officialPin(pin) {
  assert(
    pin.abi_version === null || Number(pin.abi_version.split('.')[0]) === pin.driver_family,
    'DRIVER_ABI',
  );
  assert(
    pin.os !== 'linux' || pin.abi_version === null || pin.abi_version === pin.version,
    'DRIVER_ABI',
  );
  assert(
    pin.version === null || Number(pin.version.split('.')[0]) === pin.driver_family,
    'DRIVER_SCOPE',
  );
  if (pin.url === null) {
    assert(
      pin.status === 'UNPINNED' && pin.sha256 === null && pin.size === null,
      'UNPINNED_DRIVER',
    );
    return;
  }
  assert(pin.version !== null, 'DRIVER_URL');
  let u;
  try {
    u = new URL(pin.url);
  } catch {
    assert(false, 'DRIVER_URL');
  }
  assert(
    u.protocol === 'https:' &&
      !u.username &&
      !u.password &&
      !u.search &&
      !u.hash &&
      !u.port &&
      !pin.url.includes('%'),
    'DRIVER_URL',
  );
  const nvidia = [
    'download.nvidia.com',
    'us.download.nvidia.com',
    'international.download.nvidia.com',
  ];
  if (nvidia.includes(u.hostname)) {
    assert(
      u.pathname.includes('/' + pin.version + '/') &&
        u.pathname.split('/').at(-1).includes(pin.version) &&
        u.pathname.endsWith(pin.os === 'linux' ? '.run' : '.exe'),
      'DRIVER_URL',
    );
    assert(pin.catalog_url === null, 'DRIVER_URL');
  } else {
    assert(
      pin.os === 'windows' &&
        ['catalog.s.download.windowsupdate.com', 'download.windowsupdate.com'].includes(u.hostname),
      'DRIVER_URL',
    );
    let c;
    try {
      c = new URL(pin.catalog_url);
    } catch {
      assert(false, 'DRIVER_URL');
    }
    assert(
      c.protocol === 'https:' &&
        c.hostname === 'www.catalog.update.microsoft.com' &&
        !c.port &&
        !c.username &&
        !c.password &&
        !c.hash &&
        c.pathname === '/ScopedViewInline.aspx' &&
        /^\?updateid=[a-f0-9-]{36}$/i.test(c.search),
      'DRIVER_URL',
    );
  }
  assert(
    pin.status !== 'PINNED' ||
      (/^[a-f0-9]{64}$/.test(pin.sha256 ?? '') && Number.isSafeInteger(pin.size) && pin.size > 0),
    'UNPINNED_DRIVER',
  );
  assert(
    pin.status !== 'UNPINNED' || (pin.sha256 === null && pin.size === null),
    'UNPINNED_DRIVER',
  );
}
export function prepare(input) {
  assert(input && Array.isArray(input.boxes) && input.boxes.length <= 5000, 'SCHEMA');
  checkSchema('input', { ...input, boxes: [] });
  const { catalog, requirements, limits } = input;
  const ctx = {
    input,
    catalog,
    requirements,
    limits,
    runs: unique(catalog.runs),
    artifacts: unique(catalog.artifacts),
    dies: unique(catalog.dies),
    pins: unique(catalog.driver_pins),
    sets: unique(catalog.driver_sets),
    profiles: unique(catalog.profiles, (p) => p.tier),
    priceCaps: unique(limits.price_caps, (p) => p.tier),
  };
  unique(requirements.items);
  assert(limits.margin_minutes < limits.hard_minutes, 'TIME_LIMIT');
  for (const p of catalog.driver_pins) officialPin(p);
  for (const a of catalog.artifacts) {
    const r = ctx.runs.get(a.producer);
    assert(r && r.setup && r.produces.includes(a.id), 'ARTIFACT_PRODUCER');
    assert(
      a.reuse_policy === 'trusted_content'
        ? a.trusted_content !== null &&
            a.trusted_content.source_revision === input.source_revision &&
            a.trusted_content.size <= a.gb * 1000000000
        : a.trusted_content === null,
      'ARTIFACT_TRUST',
    );
  }
  for (const r of catalog.runs) {
    unique(r.needs, (x) => x);
    unique(r.produces, (x) => x);
    unique(r.includes_setup, (x) => x);
    unique(r.capabilities, (x) => x);
    unique(r.driver_axes, (x) => x);
    assert(r.scope !== 'driver' || r.driver_axes.length > 0, 'DRIVER_SCOPE');
    for (const a of r.needs) assert(ctx.artifacts.has(a), 'MISSING_ARTIFACT', a);
    for (const a of r.produces)
      assert(ctx.artifacts.get(a)?.producer === r.id, 'ARTIFACT_PRODUCER');
    assert(new Set(r.produces.map((a) => ctx.artifacts.get(a).scope)).size <= 1, 'ARTIFACT_SCOPE');
    for (const a of r.includes_setup) assert(r.needs.includes(a), 'SETUP_CREDIT');
    assert(
      !r.host_swap ||
        (!r.setup && r.produces.length === 0 && r.capabilities.includes('can_swap_host_driver')),
      'HOST_SWAP',
    );
    resolveDependencies(catalog, [r.id]);
    assert(ownMinutes(ctx, r) >= 0, 'SETUP_CREDIT');
  }
  const transition = ctx.runs.get(limits.host_transition_run);
  assert(transition?.host_swap, 'HOST_TRANSITION');
  for (const r of catalog.runs.filter((r) => r.host_swap)) {
    const prerequisites = resolveDependencies(
      catalog,
      r.needs.map((a) => ctx.artifacts.get(a).producer),
    );
    assert(
      prerequisites.every(
        (id) => ctx.runs.get(id).driver_axes.length === 0 && !ctx.runs.get(id).host_swap,
      ),
      'HOST_TRANSITION',
    );
  }
  for (const d of catalog.dies) {
    unique(d.supported_branches, (x) => x);
  }
  for (const d of catalog.driver_sets) {
    unique(d.supported_dies, (x) => x);
    checkDrivers(ctx, d.drivers);
    assert(d.kind !== 'branch' || d.id === String(d.branch), 'DRIVER_SCOPE');
    for (const die of d.supported_dies) assert(ctx.dies.has(die), 'UNKNOWN_DIE');
  }
  for (const p of catalog.profiles) {
    assert(p.disk_gb >= p.min_disk_gb, 'DISK_LIMIT');
    assert(ctx.priceCaps.has(p.tier), 'PRICE_CAP');
    unique(p.artifact_sources, (x) => x.artifact);
    for (const a of p.artifact_sources) assert(ctx.artifacts.has(a.artifact), 'MISSING_ARTIFACT');
  }
  for (const q of requirements.items) {
    assert(Object.hasOwn(q, 'die') !== Object.hasOwn(q, 'die_family'), 'REQUIREMENT_TARGET');
    assert(Object.hasOwn(q, 'driver') !== Object.hasOwn(q, 'driver_sweep'), 'REQUIREMENT_DRIVER');
    assert(ctx.profiles.has(q.tier), 'UNKNOWN_TIER');
    assert(
      q.die ? ctx.dies.has(q.die) : catalog.dies.some((d) => d.die_family === q.die_family),
      'UNKNOWN_DIE',
    );
    for (const r of q.runs) assert(ctx.runs.has(r), 'UNKNOWN_RUN', r);
    if (q.driver_sweep) unique(q.driver_sweep.branches ?? q.driver_sweep.patches, (x) => x);
    if (q.driver) checkDrivers(ctx, q.driver);
    else
      for (const key of q.driver_sweep.branches ?? q.driver_sweep.patches) {
        const d = ctx.sets.get(key);
        assert(
          d && d.kind === (q.driver_sweep.branches ? 'branch' : 'patch'),
          'UNKNOWN_DRIVER_SET',
        );
      }
  }
  ctx.tokens = expandRequirements(ctx);
  const rejected = [],
    valid = [],
    byId = new Map();
  for (const b of input.boxes) {
    try {
      checkSchema('box', b);
      assert(ctx.dies.has(b.die), 'UNKNOWN_DIE');
      assert(
        b.host_driver === null
          ? b.capabilities.can_swap_host_driver
          : ctx.pins.get(b.host_driver)?.os === 'linux',
        'UNKNOWN_DRIVER',
      );
      assert(
        !limits.bad_machine_ids.includes(b.machine_id) && !limits.bad_host_ids.includes(b.host_id),
        'BAD_IDENTITY',
      );
      if (byId.has(b.id)) {
        assert(same(byId.get(b.id), b), 'AMBIGUOUS_ID');
        continue;
      }
      byId.set(b.id, b);
      valid.push(b);
    } catch (e) {
      if (!(e instanceof PlanError)) throw e;
      rejected.push({
        box_id:
          typeof b?.id === 'string' && /^[A-Za-z0-9_.:-]{1,100}$/.test(b.id) ? b.id : 'unknown',
        reason: e.code,
      });
      if (e.code === 'AMBIGUOUS_ID') byId.set(b.id, null);
    }
  }
  ctx.boxes = sorted(valid.filter((b) => byId.get(b.id) !== null));
  ctx.boxMap = new Map(ctx.boxes.map((b) => [b.id, b]));
  ctx.rejected = rejected.sort((a, b) => compare(canonical(a), canonical(b)));
  return ctx;
}
export function checkDrivers(ctx, d) {
  assert(
    ctx.pins.get(d.host)?.os === 'linux' &&
      ctx.pins.get(d.linux_guest)?.os === 'linux' &&
      ctx.pins.get(d.windows_guest)?.os === 'windows',
    'UNKNOWN_DRIVER',
  );
}
export function driverSupported(ctx, die, drivers) {
  const d = ctx.dies.get(die);
  if (!d || d.status === 'unsupported') return false;
  return Object.entries(drivers).every(([axis, id]) => {
    const p = ctx.pins.get(id);
    if (!p) return false;
    const branch = p.driver_family;
    return (
      d.supported_branches.includes(branch) &&
      (d.minimum_driver_branches[axis] === null || branch >= d.minimum_driver_branches[axis])
    );
  });
}
const approvedDie = (d) =>
  Boolean(
    d &&
      d.status === 'executable' &&
      Object.values(d.minimum_driver_branches).every((n) => n !== null),
  );
const approvedInputs = (ctx) =>
  ctx.catalog.status === 'executable' &&
  ctx.requirements.status === 'executable' &&
  ctx.limits.status === 'executable';
export function executableData(ctx, assignments = []) {
  return (
    approvedInputs(ctx) &&
    ctx.catalog.driver_pins.every((p) => p.status === 'PINNED') &&
    ctx.tokens.every((t) =>
      Object.values(t.driver).every((id) => ctx.pins.get(id).abi_version !== null),
    ) &&
    ctx.tokens.every((t) =>
      ctx.catalog.dies.some((d) => matchesTarget(ctx, t, d.id) && approvedDie(d)),
    ) &&
    assignments.every((a) => approvedDie(ctx.dies.get(a.die)))
  );
}
export function expandRequirements(ctx) {
  const out = [];
  for (const [qi, q] of sorted(ctx.requirements.items).entries()) {
    const sets = q.driver
      ? [{ id: 'primary', drivers: q.driver, supported_dies: ctx.catalog.dies.map((d) => d.id) }]
      : [...(q.driver_sweep.branches ?? q.driver_sweep.patches)]
          .sort()
          .map((id) => ctx.sets.get(id));
    for (const [di, s] of sets.entries())
      for (const [ri, run_id] of q.runs.entries()) {
        if (q.driver_sweep)
          for (const axis of ctx.runs.get(run_id).driver_axes)
            assert(ctx.pins.get(s.drivers[axis]).driver_family === s.branch, 'DRIVER_SCOPE');
        out.push({
          key: `q${qi}-d${di}-r${ri}`,
          requirement_id: q.id,
          run_index: ri,
          driver_set_id: s.id,
          run_id,
          driver: s.drivers,
          supported_dies: s.supported_dies,
          target: q.die ? { die: q.die } : { die_family: q.die_family },
          tier: q.tier,
          evidence: q.evidence,
        });
      }
  }
  assert(out.length <= 1024, 'BOUNDS');
  return out;
}
export function resolveDependencies(catalog, requested, { available = [] } = {}) {
  const runs = new Map(catalog.runs.map((r) => [r.id, r])),
    artifacts = new Map(catalog.artifacts.map((a) => [a.id, a]));
  const have = new Set(available),
    active = new Set(),
    doneSetup = new Set(),
    order = [];
  const visit = (id) => {
    const r = runs.get(id);
    assert(r, 'UNKNOWN_RUN', id);
    assert(!active.has(id), 'DEPENDENCY_CYCLE', id);
    if (r.setup && doneSetup.has(id)) return;
    active.add(id);
    for (const need of [...r.needs].sort())
      if (!have.has(need)) {
        const a = artifacts.get(need);
        assert(a && runs.has(a.producer), 'MISSING_ARTIFACT', need);
        visit(a.producer);
        assert(have.has(need), 'MISSING_ARTIFACT', need);
      }
    active.delete(id);
    order.push(id);
    if (r.setup) doneSetup.add(id);
    for (const a of r.produces) have.add(a);
  };
  for (const id of [...requested].sort(
    (a, b) => Number(!runs.get(a)?.host_swap) - Number(!runs.get(b)?.host_swap) || compare(a, b),
  ))
    visit(id);
  return order;
}
export function ownMinutes(ctx, r) {
  return round(
    r.estimated_minutes -
      [...new Set(r.includes_setup.map((id) => ctx.artifacts.get(id).producer))].reduce(
        (sum, id) => sum + ownMinutes(ctx, ctx.runs.get(id)),
        0,
      ),
  );
}
export const artifactKey = (ctx, id, drivers) =>
  id + '@' + (ctx.artifacts.get(id).scope === 'driver' ? driversKey(drivers) : 'box');
export function buildSteps(ctx, tokens) {
  ctx.builtCache ??= new Map();
  const cacheKey = tokens
    .map((t) => t.key)
    .sort()
    .join('|');
  if (ctx.builtCache.has(cacheKey)) return ctx.builtCache.get(cacheKey);
  const steps = [],
    produced = new Map(),
    active = new Set();
  const visit = (id, drivers, coverage) => {
    const r = ctx.runs.get(id),
      key = id + '@' + driversKey(drivers);
    assert(!active.has(key), 'DEPENDENCY_CYCLE');
    const existing =
      r.setup &&
      r.produces.length &&
      r.produces.map((a) => produced.get(artifactKey(ctx, a, drivers))).find(Boolean);
    if (existing && r.produces.every((a) => produced.has(artifactKey(ctx, a, drivers)))) {
      if (coverage) existing.coverage.push(coverage);
      return existing;
    }
    active.add(key);
    for (const need of [...r.needs].sort())
      if (!produced.has(artifactKey(ctx, need, drivers)))
        visit(ctx.artifacts.get(need).producer, drivers, null);
    const step = {
      id: 'step-' + String(steps.length + 1).padStart(3, '0'),
      run_id: id,
      drivers: { ...drivers },
      parameters: JSON.parse(canonical(r.parameters)),
      minutes: ownMinutes(ctx, r),
      needs: [...r.needs].sort().map((a) => artifactKey(ctx, a, drivers)),
      produces: [...r.produces].sort().map((a) => artifactKey(ctx, a, drivers)),
      coverage: coverage ? [coverage] : [],
      optional_seed_for: null,
    };
    steps.push(step);
    for (const a of r.produces) produced.set(artifactKey(ctx, a, drivers), step);
    active.delete(key);
    return step;
  };
  const ordered = [...tokens].sort(
    (a, b) =>
      Number(a.evidence !== 'primary') - Number(b.evidence !== 'primary') ||
      compare(driversKey(a.driver), driversKey(b.driver)) ||
      Number(!ctx.runs.get(a.run_id).host_swap) - Number(!ctx.runs.get(b.run_id).host_swap) ||
      compare(a.run_id, b.run_id) ||
      compare(a.key, b.key),
  );
  const consumed = new Set();
  let activeHost = null;
  for (const t of ordered) {
    if (activeHost !== null && activeHost !== t.driver.host) {
      const swap = ordered.find(
        (x) =>
          !consumed.has(x.key) &&
          driversKey(x.driver) === driversKey(t.driver) &&
          ctx.runs.get(x.run_id).host_swap,
      );
      if (swap) {
        visit(swap.run_id, swap.driver, swap.key);
        consumed.add(swap.key);
      } else visit(ctx.limits.host_transition_run, t.driver, null);
    }
    activeHost = t.driver.host;
    if (!consumed.has(t.key)) {
      visit(t.run_id, t.driver, t.key);
      consumed.add(t.key);
    }
  }

  for (const s of steps) s.coverage.sort();
  const reuse = [];
  for (const [key, producer] of produced) {
    const consumers = steps.filter((s) => s.needs.includes(key)).map((s) => s.id);
    if (consumers.length)
      reuse.push({
        artifact: key.split('@')[0],
        scope_key: key,
        producer_step_id: producer.id,
        consumer_step_ids: consumers,
      });
  }
  const result = {
    steps,
    reuse: reuse.sort((a, b) => compare(a.scope_key, b.scope_key)),
    minutes: round(steps.reduce((n, s) => n + s.minutes, 0)),
  };
  ctx.builtCache.set(cacheKey, result);
  return result;
}
export function matchesTarget(ctx, t, die) {
  return (
    (t.target.die ? die === t.target.die : ctx.dies.get(die)?.die_family === t.target.die_family) &&
    t.supported_dies.includes(die) &&
    driverSupported(ctx, die, t.driver)
  );
}
export function profileFor(ctx, tokens) {
  return tokens
    .map((t) => ctx.profiles.get(t.tier))
    .sort((a, b) => b.rank - a.rank || compare(a.tier, b.tier))[0];
}
export function eligible(ctx, box, tokens, profile, built) {
  const p = profile,
    l = box.limits,
    q = box.pricing;
  if (!tokens.every((t) => matchesTarget(ctx, t, box.die))) return false;
  if (
    tokens.some((t) => t.target.die_family) &&
    approvedInputs(ctx) &&
    !approvedDie(ctx.dies.get(box.die))
  )
    return false;
  if (
    tokens.some((t) => t.evidence === 'primary') &&
    new Set(tokens.map((t) => t.driver.host)).size !== 1
  )
    return false;
  if (
    !box.capabilities.can_swap_host_driver &&
    tokens.some((t) => t.driver.host !== box.host_driver)
  )
    return false;
  if (
    built.steps.some((s) => ctx.runs.get(s.run_id).host_swap) &&
    tokens.some((t) => t.evidence === 'primary')
  )
    return false;
  if (
    built.steps.some((s) => ctx.runs.get(s.run_id).capabilities.some((c) => !box.capabilities[c]))
  )
    return false;
  if (
    p.disk_gb < Math.max(...built.steps.map((s) => ctx.runs.get(s.run_id).disk_gb)) ||
    l.disk_gb < p.disk_gb ||
    l.disk_gb < p.min_disk_gb
  )
    return false;
  if (
    l.cpu_cores < p.min_cpu_cores ||
    l.disk_mb_per_second < p.min_disk_mb_per_second ||
    l.inet_down_mbps < p.min_down_mbps ||
    l.inet_up_mbps < p.min_up_mbps ||
    l.reliability < p.min_reliability ||
    l.dlperf_ratio < p.min_dlperf_ratio
  )
    return false;
  if (q.inet_down_usd_gb > p.max_inet_usd_gb || q.inet_up_usd_gb > p.max_inet_usd_gb) return false;
  if (
    built.minutes >
    Math.min(ctx.limits.hard_minutes, l.hard_minutes) - ctx.limits.margin_minutes + 1e-7
  )
    return false;
  return true;
}
export function quote(
  ctx,
  box,
  profile,
  built,
  { uploadGb = profile.base_upload_gb, minutes = built.minutes, downloadExtraGb = 0 } = {},
) {
  const hard = Math.min(ctx.limits.hard_minutes, box.limits.hard_minutes),
    p = box.pricing;
  const down = round(
      Math.max(
        profile.bandwidth_down_gb,
        built.steps.reduce((sum, s) => sum + ctx.runs.get(s.run_id).download_gb, 0),
      ) + downloadExtraGb,
    ),
    up = round(uploadGb);
  const hourly = round(
    p.compute_usd_hour +
      (p.storage_usd_gb_month * profile.disk_gb) / ctx.limits.storage_month_hours,
  );
  const transfer = round(down * p.inet_down_usd_gb + up * p.inet_up_usd_gb);
  return {
    disk_gb: profile.disk_gb,
    bandwidth_down_gb: round(down),
    bandwidth_up_gb: up,
    hourly_usd: round(hourly),
    transfer_usd: round(transfer),
    effective_usd_hour: round(hourly + transfer / (hard / 60)),
    estimated_usd: round((hourly * minutes) / 60 + transfer),
    reserved_usd: round((hourly * hard) / 60 + transfer),
  };
}
export const withinPrice = (ctx, profile, quote) =>
  quote.effective_usd_hour <= ctx.priceCaps.get(profile.tier).usd_hour;
