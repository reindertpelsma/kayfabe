#!/usr/bin/env node
// Every fault must be killed by >=2 named tests, never syntax/import/setup errors.
import { cpSync, mkdtempSync, readFileSync, writeFileSync, mkdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = dirname(dirname(fileURLToPath(import.meta.url))),
  scratch = mkdtempSync(join(tmpdir(), 'kf-J2-planner-mutants-'));
const files = ['planner', 'contracts', 'seeding', 'dependency-vectors', 'purity'].map(
  (x) => 'test/' + x + '.test.js',
);
const one = (name, file, before, after, pattern) => ({
  name,
  edits: [{ file, before, after }],
  pattern,
});
const cases = [
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
      if (/published schema|pure API/.test(c.pattern)) selectedFiles = ['test/purity.test.js'];
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
