// Pure protocol reducer. Authentication, fresh provider facts, D1 transactions,
// rentals and HTTP transport belong to the future Worker route adapter.
import { assert, checkSchema, canonical, same, compare } from './schema.js';
import { validatePlan, validateAssignment } from './validate.js';
import { round } from './model.js';
import ADDRESS_POLICY from './endpoint-policy.json' with { type: 'json' };
const copy = (x) => JSON.parse(canonical(x));
const runtime = (s, id) => s.assignments.find((a) => a.assignment_id === id);
const planned = (c, id) => c.plan.assignments.find((a) => a.id === id);
function clock(state, now) {
  assert(Number.isFinite(now) && now >= state.now_minute && now <= 100000, 'PROTOCOL_CLOCK');
}
function context(c, state, now) {
  validatePlan(c.input, c.plan);
  checkSchema('protocol_state', state);
  checkSchema('protocol_auth', c.auth);
  clock(state, now);
  assert(
    state.session_id === c.session_id && c.auth.session_id === state.session_id,
    'PROTOCOL_IDENTITY',
  );
  assert(
    same(
      state.assignments.map((a) => a.assignment_id).sort(),
      c.plan.assignments.map((a) => a.id).sort(),
    ),
    'PROTOCOL_STATE',
  );
  const a = planned(c, c.auth.assignment_id),
    r = runtime(state, c.auth.assignment_id);
  assert(
    a && r && r.instance_id === c.auth.instance_id && r.status === 'running',
    'PROTOCOL_IDENTITY',
  );
  assert(
    r.started_minute !== null &&
      r.started_minute <= now &&
      now < r.started_minute + a.reservation_minutes &&
      now < c.auth.expires_minute,
    'PROTOCOL_EXPIRED',
  );
  for (const row of state.artifacts) {
    const producer = planned(c, row.producer_assignment_id);
    assert(
      producer?.seed_server &&
        producer.seed_exports.some(
          (e) => e.scope_key === row.scope_key && e.artifact === row.artifact,
        ),
      'PROTOCOL_STATE',
    );
    assert(row.serve_until_minute === producer.seed_server.serve_until_minute, 'PROTOCOL_STATE');
    if (row.state === 'ready')
      assert(
        row.url !== null &&
          row.sha256 !== null &&
          row.size !== null &&
          row.ready_minute !== null &&
          row.gone_reason === null,
        'PROTOCOL_STATE',
      );
  }
  assert(
    new Set(state.artifacts.map((a) => canonical([a.producer_assignment_id, a.scope_key]))).size ===
      state.artifacts.length,
    'PROTOCOL_STATE',
  );
  assert(
    new Set(
      state.consumer_failures.map((x) =>
        canonical([x.consumer_assignment_id, x.producer_assignment_id, x.scope_key]),
      ),
    ).size === state.consumer_failures.length,
    'PROTOCOL_STATE',
  );
  for (const failure of state.consumer_failures) {
    const consumer = planned(c, failure.consumer_assignment_id);
    assert(
      consumer?.seed_hints.some(
        (h) =>
          h.producer_assignment_id === failure.producer_assignment_id &&
          h.scope_key === failure.scope_key,
      ),
      'PROTOCOL_STATE',
    );
  }
  return { a, r };
}
function finish(state, now) {
  state.now_minute = now;
  state.revision++;
  checkSchema('protocol_state', state);
  return copy(state);
}
function one(items, name) {
  const found = [
    ...new Map(
      items.filter((a) => a.artifact === name || a.scope_key === name).map((a) => [a.scope_key, a]),
    ).values(),
  ];
  assert(found.length === 1, 'ARTIFACT_NAME');
  return found[0];
}
function ipv4Number(ip) {
  if (!/^(?:\d{1,3}\.){3}\d{1,3}$/.test(ip)) return null;
  const n = ip.split('.').map(Number);
  if (!n.every((x) => x <= 255) || n.join('.') !== ip) return null;
  return n.reduce((v, x) => v * 256 + x, 0);
}
function publicV4(ip) {
  const n = ipv4Number(ip);
  return (
    n !== null &&
    !ADDRESS_POLICY.ipv4_deny.some((cidr) => {
      const [net, bits] = cidr.split('/'),
        unit = 2 ** (32 - Number(bits));
      return Math.floor(n / unit) === Math.floor(ipv4Number(net) / unit);
    })
  );
}
function ipv6Words(ip) {
  const [left, right] = ip.split('::');
  const l = left ? left.split(':') : [],
    r = right ? right.split(':') : [];
  const words = ip.includes('::') ? [...l, ...Array(8 - l.length - r.length).fill('0'), ...r] : l;
  return words.map((x) => parseInt(x, 16));
}
function publicIp(ip) {
  if (!ip.includes(':')) return publicV4(ip);
  try {
    // URL parsing rejects malformed literals and zone IDs. Require normalized provider facts.
    if (new URL('http://[' + ip + ']/').hostname !== '[' + ip + ']') return false;
    const words = ipv6Words(ip);
    if (words.slice(0, 5).every((x) => x === 0) && words[5] === 0xffff)
      return publicV4([words[6] >> 8, words[6] & 255, words[7] >> 8, words[7] & 255].join('.'));
    const number = words.reduce((v, x) => (v << 16n) + BigInt(x), 0n);
    const inRange = (cidr) => {
      const [net, bits] = cidr.split('/');
      const prefix = ipv6Words(net).reduce((v, x) => (v << 16n) + BigInt(x), 0n);
      return number >> BigInt(128 - Number(bits)) === prefix >> BigInt(128 - Number(bits));
    };
    return inRange(ADDRESS_POLICY.ipv6_unicast) && !ADDRESS_POLICY.ipv6_deny.some(inRange);
  } catch {
    return false;
  }
}
function endpoint(c, a, body, now) {
  checkSchema('provider_facts', c.facts);
  const f = c.facts;
  assert(
    f.assignment_id === a.id && f.instance_id === c.auth.instance_id && f.provider === a.provider,
    'ARTIFACT_INSTANCE',
  );
  assert(
    f.observed_minute <= now &&
      now - f.observed_minute <= c.input.limits.seeding.provider_facts_max_age_minutes,
    'ARTIFACT_FACTS_STALE',
  );
  assert(a.capabilities.can_expose_port === true && a.seed_server, 'ARTIFACT_PORT');
  assert(publicIp(f.public_ip), 'ARTIFACT_IP');
  let u;
  try {
    u = new URL(body.url);
  } catch {
    assert(false, 'ARTIFACT_URL');
  }
  assert(
    ['http:', 'https:'].includes(u.protocol) &&
      !u.username &&
      !u.password &&
      !u.search &&
      !u.hash &&
      !body.url.includes('%') &&
      body.url === u.href,
    'ARTIFACT_URL',
  );
  const host = u.hostname.startsWith('[') ? u.hostname.slice(1, -1) : u.hostname;
  assert(host === f.public_ip, 'ARTIFACT_IP');
  const port = Number(u.port || (u.protocol === 'https:' ? 443 : 80));
  assert(
    f.requested_ports.includes(a.seed_server.container_port) &&
      f.mapped_ports.some(
        (p) =>
          p.container_port === a.seed_server.container_port &&
          p.public_port === port &&
          p.protocol === 'tcp',
      ),
    'ARTIFACT_PORT',
  );
  assert(
    /^\/[A-Za-z0-9_-]{8,128}\/[A-Za-z0-9_.-]+$/.test(u.pathname) &&
      !['.', '..'].includes(u.pathname.split('/').at(-1)),
    'ARTIFACT_PATH',
  );
}
export function createSession(plan, session_id) {
  checkSchema('plan', plan);
  const state = {
    version: 1,
    revision: 0,
    session_id,
    now_minute: 0,
    artifacts: [],
    consumer_failures: [],
    assignments: plan.assignments.map((a) => ({
      assignment_id: a.id,
      instance_id: null,
      status: 'pending',
      started_minute: null,
      ended_minute: null,
    })),
  };
  checkSchema('protocol_state', state);
  return copy(state);
}
export function startAssignment(state, id, instance_id, now) {
  checkSchema('protocol_state', state);
  clock(state, now);
  const out = copy(state),
    r = runtime(out, id);
  assert(r && ['pending', 'starting'].includes(r.status), 'PROTOCOL_TRANSITION');
  assert(!out.assignments.some((x) => x.instance_id === instance_id), 'PROTOCOL_IDENTITY');
  r.instance_id = instance_id;
  r.status = 'running';
  r.started_minute = now;
  return finish(out, now);
}
export function endAssignment(state, id, now) {
  checkSchema('protocol_state', state);
  clock(state, now);
  const out = copy(state),
    r = runtime(out, id);
  assert(r, 'PROTOCOL_IDENTITY');
  if (r.status === 'ended') return out;
  r.status = 'ended';
  r.ended_minute = now;
  for (const row of out.artifacts.filter(
    (a) => a.producer_assignment_id === id && a.state === 'ready',
  )) {
    row.state = 'gone';
    row.gone_reason = 'ended';
  }
  return finish(out, now);
}
function resolved(c, state, a, r, h, now) {
  const producer = planned(c, h.producer_assignment_id),
    live = runtime(state, producer.id);
  const deadline = Math.min(
    h.deadline_minute,
    r.started_minute + c.input.limits.seeding.max_wait_minutes,
    c.auth.expires_minute - h.transfer_minutes,
    producer.seed_server.serve_until_minute - h.transfer_minutes,
  );
  const row = state.artifacts.find(
    (x) => x.producer_assignment_id === producer.id && x.scope_key === h.scope_key,
  );
  const rejected = state.consumer_failures.some(
    (x) =>
      x.consumer_assignment_id === a.id &&
      x.producer_assignment_id === producer.id &&
      x.scope_key === h.scope_key,
  );
  const expired =
    now > deadline ||
    now + h.transfer_minutes > producer.seed_server.serve_until_minute ||
    live.status === 'ended';
  const status =
    rejected || expired || row?.state === 'gone'
      ? 'gone'
      : row?.state === 'ready'
        ? 'ready'
        : 'waiting';
  const out = {
    version: 1,
    artifact: h.artifact,
    scope_key: h.scope_key,
    server_minute: now,
    state: status,
    url: status === 'ready' ? row.url : null,
    sha256: status === 'ready' ? row.sha256 : null,
    size: status === 'ready' ? row.size : null,
    deadline_minute: Math.max(0, round(deadline)),
    fetch_until_minute: Math.max(0, round(deadline + h.transfer_minutes)),
    fallback: 'build_local',
  };
  checkSchema('artifact_response', out);
  return out;
}
export function artifactStatus(c, state, query, now) {
  const { a, r } = context(c, state, now);
  checkSchema('artifact_query', query);
  return resolved(c, state, a, r, one(a.seed_hints, query.name), now);
}
export function artifactReady(c, state, body, now) {
  const { a } = context(c, state, now);
  checkSchema('artifact_ready', body);
  endpoint(c, a, body, now);
  const exported = one(a.seed_exports, body.artifact),
    spec = c.input.catalog.artifacts.find((x) => x.id === exported.artifact);
  assert(spec?.seedable, 'ARTIFACT_NAME');
  assert(
    spec.reuse_policy === 'trusted_content' &&
      spec.trusted_content !== null &&
      body.sha256 === spec.trusted_content.sha256 &&
      body.size === spec.trusted_content.size,
    'ARTIFACT_TRUST',
  );
  assert(body.size <= spec.gb * 1000000000, 'ARTIFACT_SIZE');
  assert(now < a.seed_server.serve_until_minute, 'ARTIFACT_EXPIRED');
  const out = copy(state),
    old = out.artifacts.find(
      (x) => x.producer_assignment_id === a.id && x.scope_key === exported.scope_key,
    );
  if (old) {
    assert(old.state === 'ready', 'ARTIFACT_GONE');
    assert(
      old.url === body.url && old.sha256 === body.sha256 && old.size === body.size,
      'ARTIFACT_CONFLICT',
    );
  } else
    out.artifacts.push({
      artifact: exported.artifact,
      scope_key: exported.scope_key,
      producer_assignment_id: a.id,
      url: body.url,
      sha256: body.sha256,
      size: body.size,
      state: 'ready',
      ready_minute: now,
      serve_until_minute: a.seed_server.serve_until_minute,
      gone_reason: null,
    });
  const starts = [];
  let active = out.assignments.filter((x) => ['starting', 'running'].includes(x.status)).length;
  for (const target of [...c.plan.assignments].sort(
    (x, y) => x.start_minute - y.start_minute || compare(x.id, y.id),
  )) {
    const pending = runtime(out, target.id);
    if (c.admission_open !== true || active >= c.input.limits.max_concurrent_boxes) break;
    if (pending.status !== 'pending') continue;
    const hint = target.seed_hints.find(
      (h) => h.producer_assignment_id === a.id && h.scope_key === exported.scope_key,
    );
    if (
      !hint ||
      now > hint.deadline_minute ||
      now + hint.transfer_minutes > a.seed_server.serve_until_minute
    )
      continue;
    pending.status = 'starting';
    starts.push(target.id);
    active++;
  }
  return { state: finish(out, now), start_requests: starts };
}
export function artifactGone(c, state, body, now) {
  const { a } = context(c, state, now);
  checkSchema('artifact_gone', body);
  const out = copy(state);
  let producer = a,
    scope;
  if (['hash_mismatch', 'size_mismatch'].includes(body.reason)) {
    const hint = one(a.seed_hints, body.artifact);
    producer = planned(c, hint.producer_assignment_id);
    scope = hint.scope_key;
  } else scope = one(a.seed_exports, body.artifact).scope_key;
  const row = out.artifacts.find(
    (x) => x.producer_assignment_id === producer.id && x.scope_key === scope,
  );
  if (['hash_mismatch', 'size_mismatch'].includes(body.reason)) {
    assert(row?.state === 'ready', 'ARTIFACT_GONE');
    assert(
      body.reason === 'hash_mismatch'
        ? body.observed_sha256 !== null && body.observed_sha256 !== row.sha256
        : body.observed_size !== null && body.observed_size !== row.size,
      'ARTIFACT_MISMATCH',
    );
    // A consumer observation is not evidence about any other consumer's bytes.
    if (
      !out.consumer_failures.some(
        (x) =>
          x.consumer_assignment_id === a.id &&
          x.producer_assignment_id === producer.id &&
          x.scope_key === scope,
      )
    )
      out.consumer_failures.push({
        consumer_assignment_id: a.id,
        producer_assignment_id: producer.id,
        scope_key: scope,
        reason: body.reason,
      });
    return finish(out, now);
  }
  if (row) {
    row.state = 'gone';
    row.gone_reason = body.reason;
  } else {
    const exported = one(producer.seed_exports, body.artifact);
    out.artifacts.push({
      artifact: exported.artifact,
      scope_key: scope,
      producer_assignment_id: producer.id,
      url: null,
      sha256: null,
      size: null,
      state: 'gone',
      ready_minute: null,
      serve_until_minute: producer.seed_server.serve_until_minute,
      gone_reason: body.reason,
    });
  }
  return finish(out, now);
}
export function verifyArtifact(expected, observed) {
  assert(
    /^[a-f0-9]{64}$/.test(expected.sha256) &&
      Number.isSafeInteger(expected.size) &&
      expected.size > 0,
    'ARTIFACT_METADATA',
  );
  if (observed.size !== expected.size) return { valid: false, reason: 'size_mismatch' };
  if (observed.sha256 !== expected.sha256) return { valid: false, reason: 'hash_mismatch' };
  return { valid: true, reason: null };
}
export function assignmentResponse(c, state, mode, now) {
  const { a, r } = context(c, state, now);
  const direct = mode === 'direct';
  assert(
    mode === 'direct' || (mode === 'worker_vast' ? a.provider === 'vast' : a.provider !== 'vast'),
    'LAUNCH_PROVIDER',
  );
  if (!direct) {
    let u;
    try {
      u = new URL(c.worker_url);
    } catch {
      assert(false, 'WORKER_URL');
    }
    assert(
      u.protocol === 'https:' && u.origin === c.worker_url && !u.username && !u.password,
      'WORKER_URL',
    );
  }
  const artifacts = a.seed_hints.map((h) => resolved(c, state, a, r, h, now));
  if (direct)
    for (const item of artifacts) {
      item.state = 'gone';
      item.url = null;
      item.sha256 = null;
      item.size = null;
    }
  const result = {
    version: 1,
    assignment: copy(a),
    server_minute: now,
    started_minute: r.started_minute,
    launch: {
      mode,
      worker_url: direct ? null : c.worker_url,
      report_path: direct ? 'report.json' : null,
    },
    artifacts,
  };
  checkSchema('assignment_response', result);
  return result;
}

export function directAssignment(input, assignment, { plan: parent } = {}) {
  validateAssignment(input, assignment, { plan: parent });
  const artifacts = assignment.seed_hints.map((h) => ({
    version: 1,
    artifact: h.artifact,
    scope_key: h.scope_key,
    state: 'gone',
    server_minute: 0,
    url: null,
    sha256: null,
    size: null,
    deadline_minute: 0,
    fetch_until_minute: 0,
    fallback: 'build_local',
  }));
  const result = {
    version: 1,
    assignment: copy(assignment),
    server_minute: 0,
    started_minute: 0,
    launch: { mode: 'direct', worker_url: null, report_path: 'report.json' },
    artifacts,
  };
  checkSchema('assignment_response', result);
  return result;
}
