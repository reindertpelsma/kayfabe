#!/usr/bin/env node
// Every fault must be killed by >=2 named tests, never syntax/import/setup errors.
import { cpSync, mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = dirname(dirname(fileURLToPath(import.meta.url))),
  scratch = mkdtempSync(join(tmpdir(), 'kf-J2-planner-mutants-'));
const files = [
  'planner',
  'contracts',
  'seeding',
  'dependency-vectors',
  'purity',
  'protocol',
  'seed-lifetime',
  'review-rv2',
].map((x) => 'test/' + x + '.test.js');
const one = (name, file, before, after, pattern) => ({
  name,
  edits: [{ file, before, after }],
  pattern,
});
const cases = [
  one(
    'RV310 duplicate consumer cache accepted',
    'artifacts.js',
    'new Set(state.consumer_failures.map((x) => canonical([x.consumer_assignment_id, x.producer_assignment_id, x.scope_key]))).size === state.consumer_failures.length',
    'true',
    'RV310 duplicate consumer cache refuses',
  ),
  one(
    'RV310 foreign consumer cache accepted',
    'artifacts.js',
    'consumer?.seed_hints.some((h) => h.producer_assignment_id === failure.producer_assignment_id && h.scope_key === failure.scope_key)',
    'true',
    'RV310 foreign consumer cache refuses',
  ),
  {
    name: 'RV310 consumer claim revokes shared row',
    pattern: 'RV310 consumer isolation',
    edits: [
      {
        file: 'artifacts.js',
        before: 'return finish(out, now); } if (row) {',
        after: '} if (row) {',
      },
      {
        file: 'artifacts.js',
        before: 'row.gone_reason = body.reason;',
        after: "row.gone_reason = 'unavailable';",
      },
    ],
  },
  one(
    'RV310 shared registry accepts consumer evidence',
    'schemas/contract.schema.json',
    '"enum": ["ended", "unavailable", "expired"]',
    '"enum": ["ended", "unavailable", "expired", "hash_mismatch", "size_mismatch"]',
    'RV310 shared registry excludes consumer reports',
  ),
  one(
    'RV310 consumer failure ignored',
    'artifacts.js',
    "rejected || expired || row?.state === 'gone'",
    "expired || row?.state === 'gone'",
    'RV310 consumer isolation',
  ),
  one(
    'RV310 consumer cache loses identity',
    'artifacts.js',
    'x.consumer_assignment_id === a.id && x.producer_assignment_id === producer.id && x.scope_key === h.scope_key',
    'x.producer_assignment_id === producer.id && x.scope_key === h.scope_key',
    'RV310 consumer isolation',
  ),
  one(
    'RV310 mapped IPv4 form trusted',
    'artifacts.js',
    "return publicV4([words[6] >> 8, words[6] & 255, words[7] >> 8, words[7] & 255].join('.'));",
    'return true;',
    'RV310 endpoint refuses',
  ),
  one(
    'RV310 IPv6 unicast requirement skipped',
    'artifacts.js',
    'inRange(ADDRESS_POLICY.ipv6_unicast)',
    'true',
    'RV310 endpoint refuses',
  ),
  one(
    'RV310 IPv6 special ranges ignored',
    'artifacts.js',
    '!ADDRESS_POLICY.ipv6_deny.some(inRange)',
    'true',
    'RV310 endpoint refuses',
  ),
  one(
    'RV310 shared IPv4 allowed',
    'endpoint-policy.json',
    '"100.64.0.0/10",',
    '',
    'RV310 endpoint refuses',
  ),
  one(
    'RV310 producer selects content pins',
    'artifacts.js',
    "spec.reuse_policy === 'trusted_content' && spec.trusted_content !== null && body.sha256 === spec.trusted_content.sha256 && body.size === spec.trusted_content.size",
    'true',
    'RV310 producer cannot select trusted',
  ),
  one(
    'RV310 content revision ignored',
    'model.js',
    'a.trusted_content.source_revision === input.source_revision',
    'true',
    'RV310 content revision must match',
  ),
  one(
    'RV310 local build offers seed candidates',
    'plan.js',
    "artifact.reuse_policy !== 'trusted_content'",
    'false',
    'RV310 local build required without trusted content',
  ),
  one(
    'RV310 assignment pin replaced',
    'plan.js',
    'trusted_content: artifact.trusted_content',
    "trusted_content: {...artifact.trusted_content,sha256:'c'.repeat(64)}",
    'RV310 assignment contains trusted content',
  ),
  one(
    'RV310 catalog reuse policy optional',
    'schemas/contract.schema.json',
    '"required": ["id", "producer", "gb", "scope", "seedable", "reuse_policy", "trusted_content"]',
    '"required": ["id", "producer", "gb", "scope", "seedable", "trusted_content"]',
    'RV310 catalog requires reuse_policy',
  ),
  one(
    'RV310 catalog pin optional',
    'schemas/contract.schema.json',
    '"required": ["id", "producer", "gb", "scope", "seedable", "reuse_policy", "trusted_content"]',
    '"required": ["id", "producer", "gb", "scope", "seedable", "reuse_policy"]',
    'RV310 catalog requires trusted_content',
  ),
  one(
    'artifact URL host binding skipped',
    'artifacts.js',
    'host === f.public_ip',
    'true',
    'artifact protocol refuses host',
  ),
  one(
    'artifact mapped public port ignored',
    'artifacts.js',
    'p.public_port === port',
    'true',
    'artifact protocol refuses port',
  ),
  one(
    'artifact requested port ignored',
    'artifacts.js',
    'f.requested_ports.includes(a.seed_server.container_port)',
    'true',
    'artifact protocol refuses requested',
  ),
  one(
    'artifact provider facts freshness ignored',
    'artifacts.js',
    'now - f.observed_minute <= c.input.limits.seeding.provider_facts_max_age_minutes',
    'true',
    'artifact protocol refuses stale',
  ),
  one(
    'artifact instance binding ignored',
    'artifacts.js',
    'f.instance_id === c.auth.instance_id',
    'true',
    'artifact protocol refuses instance',
  ),
  one(
    'artifact verified run token skipped',
    'artifacts.js',
    "checkSchema('protocol_auth', c.auth);",
    ';',
    'artifact protocol refuses token',
  ),
  one(
    'artifact session binding ignored',
    'artifacts.js',
    'state.session_id === c.session_id && c.auth.session_id === state.session_id',
    'true',
    'artifact protocol cannot extend identity',
  ),
  one(
    'artifact name not checked against the plan',
    'artifacts.js',
    'const exported = one(a.seed_exports, body.artifact),',
    "const exported = one(a.seed_exports, 'open_image'),",
    'artifact protocol refuses name',
  ),
  {
    name: 'artifact exceeds priced GB',
    pattern: 'RV310 trusted bytes must fit priced allowance',
    edits: [
      { file: 'artifacts.js', before: 'body.size <= spec.gb * 1000000000', after: 'true' },
      { file: 'model.js', before: 'a.trusted_content.size <= a.gb * 1000000000', after: 'true' },
    ],
  },
  one(
    'artifact HTTP scheme ignored',
    'artifacts.js',
    "['http:', 'https:'].includes(u.protocol)",
    'true',
    'artifact protocol refuses scheme',
  ),
  one(
    'artifact URL capability path ignored',
    'artifacts.js',
    '/^\\/[A-Za-z0-9_-]{8,128}\\/[A-Za-z0-9_.-]+$/.test(u.pathname)',
    'true',
    'artifact protocol refuses path',
  ),
  one(
    'artifact public IP sanity guard ignored',
    'artifacts.js',
    'publicIp(f.public_ip)',
    'true',
    'artifact protocol refuses nonpublic provider',
  ),
  one(
    'ready POST does not dispatch consumers',
    'artifacts.js',
    'starts.push(target.id);',
    ';',
    'artifact protocol ready claims pending',
  ),
  one(
    'duplicate ready restarts an ended consumer',
    'artifacts.js',
    "pending.status !== 'pending'",
    'false',
    'artifact protocol ended consumer',
  ),
  one(
    'starting claims do not consume concurrency',
    'artifacts.js',
    "['starting', 'running'].includes(x.status)",
    "['running'].includes(x.status)",
    'artifact protocol starting claims',
  ),
  one(
    'ready bypasses live concurrency',
    'artifacts.js',
    'active >= c.input.limits.max_concurrent_boxes',
    'false',
    'artifact protocol never claims more',
  ),
  one(
    'ready bypasses closed admission',
    'artifacts.js',
    'c.admission_open !== true',
    'false',
    'artifact protocol no new starts while admission',
  ),
  one(
    'late ready starts a consumer',
    'artifacts.js',
    'now > hint.deadline_minute',
    'false',
    'artifact protocol late ready',
  ),
  one(
    'source publishes after serving lifetime',
    'artifacts.js',
    'now < a.seed_server.serve_until_minute',
    'true',
    'artifact protocol source cannot publish after',
  ),
  one(
    'gone artifact accepts a new ready POST',
    'artifacts.js',
    "old.state === 'ready'",
    'true',
    'artifact protocol gone post creates a terminal tombstone',
  ),
  one(
    'ending a source leaves its artifacts ready',
    'artifacts.js',
    "(a) => a.producer_assignment_id === id && a.state === 'ready'",
    '(a) => false',
    'artifact protocol consumer polls ready then source loss',
  ),
  one(
    'artifact hash mismatch passes verification',
    'artifacts.js',
    'observed.sha256 !== expected.sha256',
    'false',
    'artifact protocol metadata verifier checks',
  ),
  one(
    'artifact size mismatch passes verification',
    'artifacts.js',
    'observed.size !== expected.size',
    'false',
    'artifact protocol metadata verifier checks',
  ),
  one(
    'false consumer hash mismatch invalidates artifact',
    'artifacts.js',
    'body.observed_sha256 !== null && body.observed_sha256 !== row.sha256',
    'true',
    'artifact protocol false mismatch report',
  ),
  one(
    'early consumer gets an unbounded wait',
    'artifacts.js',
    'r.started_minute + c.input.limits.seeding.max_wait_minutes',
    '100000',
    'artifact protocol early consumer',
  ),
  one(
    'launch mode ignores provider',
    'artifacts.js',
    "mode === 'direct' || (mode === 'worker_vast' ? a.provider === 'vast' : a.provider !== 'vast')",
    'true',
    'artifact protocol launch mode must match',
  ),
  one(
    'DIRECT requires a Worker token',
    'artifacts.js',
    'export function directAssignment(input, assignment, { plan: parent } = {}) {',
    "export function directAssignment(input, assignment, { plan: parent } = {}) { assert(false,'PROTOCOL_IDENTITY');",
    'artifact protocol local assignment needs no Worker',
  ),
  one(
    'direct seed lifetime ignored',
    'plan.js',
    'h.deadline_minute >= a.start_minute && h.fetch_until_minute <= end + 1e-7',
    'true',
    'one concurrent box cannot consume',
  ),
  one(
    'invalid direct edge aborts cold planning',
    'plan.js',
    "if (!edges.length || !(e instanceof PlanError) || e.code !== 'SEED_LIFETIME') throw e;",
    'throw e;',
    'one concurrent box cannot consume',
  ),
  one(
    'direct serving idle time not charged',
    'plan.js',
    'source.estimated_minutes = round(end - source.start_minute);',
    'source.estimated_minutes = source.fallback_minutes;',
    'direct serving minutes and cost',
  ),
  one(
    'wait instructions omitted from assignment',
    'plan.js',
    '...a.seed_hints.map((h) => ({',
    '...[].map((h) => ({',
    'assignment has explicit optional wait',
  ),
  one(
    'seed port is hardcoded',
    'plan.js',
    'container_port: ctx.limits.seeding.container_port',
    'container_port: 8080',
    'direct seed port is a data',
  ),
  {
    name: 'portless source allowed to seed',
    edits: [
      {
        file: 'plan.js',
        before: 'source.box.capabilities.can_expose_port === true',
        after: 'true',
      },
      {
        file: 'plan.js',
        before: 'byId.get(s.id).box.capabilities.can_expose_port === true',
        after: 'true',
      },
    ],
    pattern: 'no exposed port disables',
  },
  one(
    'inherited schema field admitted',
    'schema.js',
    's.properties && Object.hasOwn(s.properties, k) ? s.properties[k] : undefined',
    's.properties?.[k]',
    'closed schema rejects inherited',
  ),
  one(
    'proposed catalog admitted',
    'model.js',
    "ctx.catalog.status === 'executable'",
    'true',
    'proposed catalog cannot|non-executable trusted data.*proposed',
  ),
  one(
    'proposed requirements admitted',
    'model.js',
    "ctx.requirements.status === 'executable'",
    'true',
    'proposed requirements cannot',
  ),
  one(
    'proposed limits admitted',
    'model.js',
    "ctx.limits.status === 'executable'",
    'true',
    'proposed limits cannot',
  ),
  one(
    'UNPINNED catalog admitted',
    'model.js',
    "ctx.catalog.driver_pins.every((p) => p.status === 'PINNED')",
    'true',
    'unpinned catalog entry',
  ),
  one(
    'unknown driver minimum executable',
    'model.js',
    'Object.values(d.minimum_driver_branches).every((n) => n !== null)',
    'true',
    'unknown selected minimum',
  ),
  one(
    'proposed die executable',
    'model.js',
    "d.status === 'executable'",
    'true',
    'proposed selected die',
  ),
  one(
    'official installer verification skipped',
    'model.js',
    'export function officialPin(pin) {',
    'export function officialPin(pin) { return;',
    'official installer URL',
  ),
  one(
    'driver OS binding skipped',
    'model.js',
    'export function checkDrivers(ctx, d) {',
    'export function checkDrivers(ctx, d) { return;',
    'selected pin OS',
  ),
  one(
    'per-die driver minimum ignored',
    'model.js',
    'branch >= d.minimum_driver_branches[axis]',
    'true',
    'unsupported/too-old driver',
  ),
  one(
    'driver branch coverage mislabeled',
    'model.js',
    'ctx.pins.get(s.drivers[axis]).driver_family === s.branch',
    'true',
    'branch sweep cannot claim',
  ),
  one(
    'fixed host mismatch ignored',
    'model.js',
    '!box.capabilities.can_swap_host_driver &&',
    'false &&',
    'fixed known host cannot claim',
  ),
  one(
    'host swap shares primary evidence',
    'model.js',
    "built.steps.some((s) => ctx.runs.get(s.run_id).host_swap) && tokens.some((t) => t.evidence === 'primary')",
    'false',
    'host swaps isolated',
  ),
  one(
    'implicit host transition omitted',
    'model.js',
    'else visit(ctx.limits.host_transition_run, t.driver, null);',
    'else {}',
    'isolated driver box uses explicit host transition',
  ),
  one(
    'driver cache delimiter collision',
    'model.js',
    'canonical([d.host, d.linux_guest, d.windows_guest])',
    "[d.host,d.linux_guest,d.windows_guest].join(':')",
    'driver cache keys cannot collide',
  ),
  one(
    'required capabilities ignored',
    'model.js',
    '.capabilities.some((c) => !box.capabilities[c])',
    '.capabilities.some((c) => false)',
    'guest capabilities remain mandatory|fixed host cannot swap',
  ),
  one(
    'allocated disk fit ignored',
    'model.js',
    'l.disk_gb < p.disk_gb',
    'false',
    'requested allocated disk',
  ),
  ...[
    ['cpu_cores', 'min_cpu_cores'],
    ['disk_mb_per_second', 'min_disk_mb_per_second'],
    ['inet_down_mbps', 'min_down_mbps'],
    ['inet_up_mbps', 'min_up_mbps'],
    ['reliability', 'min_reliability'],
    ['dlperf_ratio', 'min_dlperf_ratio'],
  ].map(([field, floor]) =>
    one(
      'quality floor ' + field + ' ignored',
      'model.js',
      `l.${field} < p.${floor}`,
      'false',
      'hard quality floor ' + field,
    ),
  ),
  one(
    'inbound unit price cap ignored',
    'model.js',
    'q.inet_down_usd_gb > p.max_inet_usd_gb',
    'false',
    'per-GB cap applies to inet_down',
  ),
  one(
    'outbound unit price cap ignored',
    'model.js',
    'q.inet_up_usd_gb > p.max_inet_usd_gb',
    'false',
    'per-GB cap applies to inet_up',
  ),
  one(
    'per-tier effective cap ignored',
    'model.js',
    'quote.effective_usd_hour <= ctx.priceCaps.get(profile.tier).usd_hour',
    'true',
    'per-tier cap refuses',
  ),
  one(
    'expected transfer price omitted',
    'model.js',
    'round(down * p.inet_down_usd_gb + up * p.inet_up_usd_gb)',
    '0',
    'network-inclusive ranking',
  ),
  one(
    'profile storage price omitted',
    'model.js',
    '(p.storage_usd_gb_month * profile.disk_gb) / ctx.limits.storage_month_hours',
    '0',
    'profile storage affects ranking',
  ),
  one(
    'large allowance replaced by small',
    'model.js',
    'profile.bandwidth_down_gb',
    '73',
    'network-inclusive ranking',
  ),
  one(
    'seeder upload priced as evidence only',
    'model.js',
    'up = round(uploadGb)',
    'up = round(profile.base_upload_gb)',
    'seeder role re-ranks|upload reservation appears',
  ),
  one(
    'reservation uses unrounded hidden hourly price',
    'model.js',
    'const hourly = round(p.compute_usd_hour + (p.storage_usd_gb_month * profile.disk_gb) / ctx.limits.storage_month_hours);',
    'const hourly = p.compute_usd_hour+(p.storage_usd_gb_month*profile.disk_gb)/ctx.limits.storage_month_hours;',
    'reservation covers its own published',
  ),
  one(
    'box count cap ignored',
    'plan.js',
    'bins.length < ctx.limits.max_boxes',
    'true',
    'maximum box count',
  ),
  one(
    'physical machine uniqueness ignored',
    'plan.js',
    'if (used.has(box.machine_id) || !eligible(ctx, box, [t], profile, built)) continue;',
    'if (!eligible(ctx, box, [t], profile, built)) continue;',
    'same physical machine cannot',
  ),
  one(
    'concurrency interval cap ignored',
    'plan.js',
    '<= cap',
    '<= Infinity',
    'concurrency offsets reserve',
  ),
  one(
    'time margin ignored at packing',
    'model.js',
    '- ctx.limits.margin_minutes + 1e-7',
    '+ 1e-7',
    'margin removes usable',
  ),
  one(
    'provider hard deadline ignored',
    'model.js',
    'Math.min(ctx.limits.hard_minutes, l.hard_minutes)',
    'ctx.limits.hard_minutes',
    'provider box hard limit',
  ),
  one(
    'reservation cap ignored on new rental',
    'plan.js',
    'reserved + price.reserved_usd <= ctx.limits.cost_cap_usd',
    'true',
    'total reservation capped',
  ),
  one(
    'authoritative plan reconstruction ignored',
    'validate.js',
    'same(p, expected)',
    'true',
    'validator refuses tampered|reject tampered seed',
  ),
  one(
    'self-run assignment binding ignored',
    'validate.js',
    'same(a, expected)',
    'true',
    'assignment cannot bypass',
  ),
  one(
    'closed schema accepts unknown fields',
    'schema.js',
    's.additionalProperties === false',
    'false',
    'closed versioned',
  ),
  one(
    'schema constants ignored',
    'schema.js',
    's.const !== undefined && !same(s.const, v)',
    'false',
    'closed versioned',
  ),
  one(
    'shared setup credit counted twice',
    'model.js',
    '[...new Set(r.includes_setup.map((id) => ctx.artifacts.get(id).producer))]',
    'r.includes_setup.map(id=>ctx.artifacts.get(id).producer)',
    'one shared producer receives only one',
  ),
  one(
    'resolver requested order not canonical',
    'model.js',
    '[...requested].sort((a,b)=>Number(!runs.get(a)?.host_swap)-Number(!runs.get(b)?.host_swap)||compare(a,b))',
    '[...requested]',
    'shared dependency vector: shared-provision',
  ),
  one(
    'resolver cycle detection removed',
    'model.js',
    "assert(!active.has(id), 'DEPENDENCY_CYCLE', id);",
    '',
    'shared dependency vector: cycle-two-runs',
  ),
  one(
    'seeder role not repriced after assignment',
    'plan.js',
    'if (!a.assigned_consumers.length) continue;',
    'if (true) continue;',
    'seeder role re-ranks',
  ),
  one(
    'seeder speed weight hardcoded zero',
    'plan.js',
    's.upload_speed_weight',
    '0',
    'seeder upload-speed preference is data',
  ),
  one(
    'seed edge removal is a no-op',
    'index.js',
    'const cold = removeEdges(ctx, p);',
    'const cold = p;',
    'removing seed edges preserves',
  ),
  one(
    'unoffered die reported as unsatisfied',
    'plan.js',
    "status: targetOffered ? 'unsatisfied' : 'not_offered'",
    "status: 'unsatisfied'",
    'die not offered is explicit',
  ),
  one(
    'die target ambiguity accepted',
    'model.js',
    "Object.hasOwn(q, 'die') !== Object.hasOwn(q, 'die_family')",
    'true',
    'requirement cannot supply die and die family',
  ),
  {
    name: 'driver target ambiguity accepted',
    pattern: 'requirement cannot supply driver and sweep',
    edits: [
      {
        file: 'model.js',
        before: "Object.hasOwn(q, 'driver') !== Object.hasOwn(q, 'driver_sweep')",
        after: 'true',
      },
      {
        file: 'model.js',
        before: 'if (q.driver_sweep) for (const axis of ctx.runs.get(run_id).driver_axes)',
        after:
          'if (q.driver_sweep && !q.driver) for (const axis of ctx.runs.get(run_id).driver_axes)',
      },
    ],
  },
  one(
    'seeded setup fallback removed',
    'plan.js',
    'const out = [...assignments.values()].sort((a, b) => lex(a.id, b.id));',
    'for(const a of assignments.values())a.runs=a.runs.filter(r=>!r.optional_seed_for); const out = [...assignments.values()].sort((a,b)=>lex(a.id,b.id));',
    'deadline and fallback reserve',
  ),
];

cases.push(
  one(
    'host transition selector hardcoded',
    'model.js',
    'visit(ctx.limits.host_transition_run,t.driver,null)',
    "visit('host_tiny',t.driver,null)",
    'host transition action is catalog data',
  ),
  one(
    'planner swap ordering ignored',
    'model.js',
    'Number(!ctx.runs.get(a.run_id).host_swap)-Number(!ctx.runs.get(b.run_id).host_swap)',
    '0',
    'host swap precedes its driver guest runs',
  ),
  one(
    'portable resolver swap ordering ignored',
    'model.js',
    'Number(!runs.get(a)?.host_swap)-Number(!runs.get(b)?.host_swap)',
    '0',
    'shared dependency vector: host-swap-before-guests',
  ),
  one(
    'selected ABI approval ignored',
    'model.js',
    'ctx.tokens.every((t)=>Object.values(t.driver).every((id)=>ctx.pins.get(id).abi_version!==null))',
    'true',
    'unknown selected guest ABI',
  ),
  one(
    'stateful host action declared cacheable',
    'model.js',
    "!r.setup && r.produces.length===0 && r.capabilities.includes('can_swap_host_driver')",
    "r.produces.length===0&&r.capabilities.includes('can_swap_host_driver')",
    'host driver action is never cacheable',
  ),
);

cases.push(
  one(
    'published schema becomes mutable',
    'schema.js',
    'freeze(contract);',
    '',
    'published schema cannot mutate',
  ),
  one(
    'planner consults clock',
    'index.js',
    'export function plan(input) {',
    'export function plan(input) { Date.now();',
    'pure API never consults',
  ),
  one(
    'planner consults randomness',
    'index.js',
    'export function plan(input) {',
    'export function plan(input) { Math.random();',
    'pure API never consults',
  ),
);
function replace(source, before, after) {
  const compact = [],
    positions = [];
  for (let i = 0; i < source.length; i++)
    if (!/\s/.test(source[i]) && !(source[i] === ',' && /^\s*[)\]}]/.test(source.slice(i + 1)))) {
      compact.push(source[i]);
      positions.push(i);
    }
  const text = compact.join(''),
    needle = before.replace(/\s/g, ''),
    at = text.indexOf(needle);
  if (at < 0 || text.indexOf(needle, at + needle.length) >= 0)
    throw Error('mutation anchor must occur exactly once: ' + before);
  return (
    source.slice(0, positions[at]) + after + source.slice(positions[at + needle.length - 1] + 1)
  );
}
function run(dir, pattern = '.', selectedFiles = files) {
  return spawnSync(
    process.execPath,
    [
      '--test',
      '--test-reporter=tap',
      '--test-name-pattern=' + pattern,
      ...selectedFiles.map((f) => join(dir, f)),
    ],
    {
      encoding: 'utf8',
      timeout: 90000,
      maxBuffer: 16 * 1024 * 1024,
      env: { ...process.env, KF_PLANNER_SEEDS: '1' },
    },
  );
}
let result = { status: 'FAIL', baseline: null, cases: [] };
try {
  const baseline = run(root);
  writeFileSync(join(scratch, 'baseline.log'), baseline.stdout + '\n' + baseline.stderr);
  if (baseline.status !== 0)
    throw Error('unmutated baseline failed: ' + baseline.stdout.slice(-3000) + baseline.stderr);
  const names = new Set([...baseline.stdout.matchAll(/^ok \d+ - (.*)$/gm)].map((m) => m[1]));
  result.baseline = { tests: names.size };
  for (const c of cases)
    if (['die target ambiguity accepted', 'driver target ambiguity accepted'].includes(c.name))
      c.edits.push({
        file: 'schema.js',
        before: 's.oneOf && s.oneOf.filter((x) => !matches(x, v, path)).length !== 1',
        after: 'false',
      });
  const selected = process.env.KF_MUTANT_FILTER
    ? cases.filter((c) => new RegExp(process.env.KF_MUTANT_FILTER).test(c.name))
    : cases;
  if (!selected.length) throw Error('empty mutation selection');
  for (const [i, c] of selected.entries()) {
    const dir = join(scratch, 'case-' + i);
    mkdirSync(dir);
    cpSync(root, join(dir, 'planner'), { recursive: true });
    writeFileSync(join(dir, 'package.json'), '{"type":"module"}\n');
    const target = join(dir, 'planner');
    let row = { mutant: c.name, kills: 0, killers: [] };
    try {
      for (const e of c.edits) {
        const path = join(target, e.file);
        writeFileSync(path, replace(readFileSync(path, 'utf8'), e.before, e.after));
      }
      let selectedFiles = ['test/contracts.test.js'];
      if (c.pattern.startsWith('RV310')) selectedFiles = ['test/review-rv2.test.js'];
      else if (c.pattern.startsWith('artifact protocol')) selectedFiles = ['test/protocol.test.js'];
      else if (
        /no exposed port|one concurrent box|direct serving|direct seed port|assignment has explicit optional wait/.test(
          c.pattern,
        )
      )
        selectedFiles = ['test/seed-lifetime.test.js'];
      else if (/published schema|pure API/.test(c.pattern)) selectedFiles = ['test/purity.test.js'];
      else if (c.pattern.includes('shared dependency vector'))
        selectedFiles = ['test/dependency-vectors.test.js'];
      else if (c.pattern.includes('validator refuses tampered'))
        selectedFiles = ['test/planner.test.js', 'test/seeding.test.js'];
      else if (
        /unsupported\/too-old|host swaps isolated|concurrency offsets reserve|total reservation capped|die not offered/.test(
          c.pattern,
        )
      )
        selectedFiles = ['test/planner.test.js'];
      else if (/seeder |seed edges|upload reservation|deadline and fallback/.test(c.pattern))
        selectedFiles = ['test/seeding.test.js'];
      const out = run(target, c.pattern, selectedFiles);
      writeFileSync(join(dir, 'test.log'), out.stdout + '\n' + out.stderr);
      row.killers = [
        ...new Set(
          [...out.stdout.matchAll(/^not ok \d+ - (.*)$/gm)]
            .map((m) => m[1])
            .filter((n) => names.has(n)),
        ),
      ];
      row.kills = row.killers.length;
      if (out.status !== 1 || out.error || row.kills < 2)
        row.error =
          out.error?.message ??
          'requires >=2 distinct named baseline tests; syntax/import/timeout is not a kill';
    } catch (e) {
      row.error = e.message;
    }
    result.cases.push(row);
    process.stderr.write(
      c.name +
        ': ' +
        row.kills +
        ' named killers' +
        (row.error ? ' ERROR ' + row.error : '') +
        '\n',
    );
  }
  result.status = result.cases.every((c) => !c.error && c.kills >= 2) ? 'PASS' : 'FAIL';
} catch (e) {
  result.error = e.message;
} finally {
  process.stdout.write(JSON.stringify(result, null, 2) + '\n');
  if (result.status === 'PASS') rmSync(scratch, { recursive: true, force: true });
  else {
    process.stderr.write('Evidence retained: ' + scratch + '\n');
    process.exitCode = 1;
  }
}
