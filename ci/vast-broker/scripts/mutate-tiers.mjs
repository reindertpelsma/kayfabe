// Mutation check for the tier, release and admin-start invariants (wtier, 2026-11-05), in the habit
// of ci/platform-api/scripts/mutate.mjs and scripts/ci/*_mutants.py. Each mutant is applied to a
// temporary copy (the checkout is never edited) and must make at least one of its named test files
// fail. Run: node ci/vast-broker/scripts/mutate-tiers.mjs [name ...]
import {
  cpSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
  mkdirSync,
  symlinkSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const broker = fileURLToPath(new URL('..', import.meta.url));
const repo = fileURLToPath(new URL('../../..', import.meta.url));
const PURE = ['tiers-release'];
const BATCH = ['batch'];
const CONFIG = ['config-booleans'];
const CLAIMS = ['tier-claims'];
const MUTANTS = [
  // the event, not a request, picks the tier
  ['PR is small', 'src/tier.js', "? 'middle' : 'small';", "? 'small' : 'small';", PURE],
  ['tag not large', 'src/tier.js', "    return 'large';", "    return 'small';", PURE],
  // configuration bounds
  [
    'release hard above check rule',
    'src/policy.js',
    "number('RELEASE_HARD_RUNTIME_MINUTES', DEFAULTS.releaseHardMinutes, DEFAULTS.hardMinutes)",
    "number('RELEASE_HARD_RUNTIME_MINUTES', DEFAULTS.releaseHardMinutes, DEFAULTS.maxMinutes)",
    PURE,
  ],
  [
    'design-only stage enabled',
    'src/policy.js',
    "DEFAULTS.stageSlots[s].state !== 'DESIGN_ONLY',",
    'true,',
    PURE,
  ],
  [
    'cost cap above daily',
    'src/policy.js',
    "if (cfg.releaseCostCap > cfg.releaseDaily) refuse('CONFIG_VAR:RELEASE_COST_CAP_USD', 503);",
    '',
    PURE,
  ],
  [
    'die moves family',
    'src/policy.js',
    'cfg.dieRules.some((r) => r.die === d && r.cc === cfg.families[f]),',
    'cfg.dieRules.some((r) => r.die === d),',
    PURE,
  ],
  [
    'second windows run on check',
    'src/tier.js',
    "if (profile !== 'release' || windowsRuns !== 2) throw Error('windows runs');",
    "if (windowsRuns !== 2) throw Error('windows runs');",
    PURE,
  ],
  // the plan
  [
    'one die per family',
    'src/release.js',
    'complete: cells.length >= cfg.releaseDiesPerFamily,',
    'complete: cells.length >= 1,',
    PURE,
  ],
  [
    'no cost cap',
    'src/release.js',
    "if (estimate > cfg.releaseCostCap) refusals.push('COST_CAP');",
    '',
    PURE,
  ],
  [
    'second leg same machine',
    'src/release.js',
    'const others = sorted.filter((o) => o.machine_id !== first.machine_id);',
    'const others = sorted.slice(1).concat(sorted);',
    PURE,
  ],
  [
    'single machine runs windows once',
    'src/release.js',
    'return [leg(first, want)];',
    'return [leg(first, 1)];',
    PURE,
  ],
  // classification and verdict
  [
    'non-PASS record passes',
    'src/release.js',
    'const finish = (parts, reason, passAllowed = false) => {',
    'const finish = (parts, reason, passAllowed = true) => {',
    PURE,
  ],
  [
    'blind fail attributed',
    'src/release.js',
    "names.some((n) => !retryRow(n)) ? (green ? 'FAIL' : 'UNATTRIBUTED') : 'RETRY';",
    "names.some((n) => !retryRow(n)) ? 'FAIL' : 'RETRY';",
    PURE,
  ],
  [
    'stop is a pass candidate',
    'src/release.js',
    "if (record.closed === 'STOPPED' && record.verdict !== 'PASS')",
    'if (false)',
    PURE,
  ],
  [
    'retry row passes',
    'src/release.js',
    "parts.windows[i] = 'PASS';\n      continue;",
    'break;',
    PURE,
  ],
  [
    'one windows pass is enough',
    'src/release.js',
    "batch.tier === 'large' ? need : 1,",
    '1,',
    PURE,
  ],
  [
    'incomplete release passes',
    'src/release.js',
    'complete && cells.length && cells.every',
    'cells.length && cells.every',
    PURE,
  ],
  [
    'FAIL re-rented',
    'src/release.js',
    "['RETRY', 'UNATTRIBUTED'].includes(leg.outcome.verdict) &&",
    "leg.outcome.verdict !== 'PASS' &&",
    PURE,
  ],
  [
    'owner stop re-rented',
    'src/release.js',
    "leg.outcome.reason !== 'STOPPED' &&",
    'true &&',
    PURE,
  ],
  // orchestration through the service
  ['no cost guard', 'src/index.js', 'if (costGuard) costGuard(cost);', '', ['batch']],
  [
    'small/middle exempt from the gap',
    'src/ledger.js',
    "batch.tier === 'large' &&",
    'true &&',
    BATCH,
  ],
  [
    'stop-all leaves releases running',
    'src/index.js',
    "        batch.state = 'STOPPED';\n        for (const l of batch.legs) if (l.state === 'PENDING') l.state = 'MISSING';",
    '',
    BATCH,
  ],
  [
    'fork commit admitted',
    'src/github.js',
    "refuse('NOT_ELIGIBLE', 409, 'NOT_ON_BRANCH');",
    "return 'fork';",
    BATCH,
  ],
  [
    'sibling may reuse machine',
    'src/index.js',
    'sibling.avoidMachines = [...new Set([...sibling.avoidMachines, started.machineId])];',
    '',
    BATCH,
  ],
  [
    'another commit answers a cell',
    'src/index.js',
    'r?.sha === batch.sha && r?.batch === batch.id ? r : undefined',
    'r',
    BATCH,
  ],
  [
    'admin leg closed as RUN_GONE',
    'src/index.js',
    "record.resultAt === undefined && record.source !== 'admin') {",
    'record.resultAt === undefined) {',
    BATCH,
  ],
  [
    'pinned die ignores avoided machines',
    'src/offers.js',
    '(x) => x.die === die && !avoidMachines.includes(x.machine_id),',
    '(x) => x.die === die,',
    BATCH,
  ],
  [
    'no VRAM floor',
    'src/offers.js',
    '        vramFloorMb > 0 &&\n        !(typeof',
    '        false &&\n        !(typeof',
    BATCH,
  ],
  [
    'rotation not stored',
    'src/index.js',
    "await ledger.put('rotation:middleFamily', chosen.family);",
    ';',
    BATCH,
  ],
  [
    'plan rents',
    'src/index.js',
    '    return { plan: await planFor(request, cfg, vast), renting: false };',
    '    await adminStart(body);\n    return { plan: await planFor(request, cfg, vast), renting: false };',
    BATCH,
  ],
  [
    'release count cap is the check cap',
    'src/ledger.js',
    'if (!options.sameAttempt && values.count_today >= caps.dailyCount)',
    'if (!options.sameAttempt && values.count_today >= cfg.dailyCount)',
    BATCH,
  ],
];

// Review fixes (rev259, 2026-11-05).
MUTANTS.push(
  [
    'renting switch not in admit',
    'src/index.js',
    "    if (env.RENTING_ENABLED !== 'true') refuse('RENTING_DISABLED', 503);\n    if (profile",
    '    if (profile',
    BATCH,
  ],
  [
    'release kill switch ignored',
    'src/index.js',
    "if (profile === 'release' && !cfg.releaseEnabled) refuse('RELEASE_DISABLED', 503);",
    ';',
    BATCH,
  ],
  [
    'start ignores the release kill switch',
    'src/index.js',
    "if (request.tier === 'large' && !cfg.releaseEnabled) refuse('RELEASE_DISABLED', 503);",
    ';',
    BATCH,
  ],
  [
    'batch stop not durable first',
    'src/index.js',
    "      for (const l of batch.legs) if (l.state === 'PENDING') l.state = 'MISSING';\n      await ledger.put('batch:' + id, batch);\n      let failed = false;",
    '      let failed = false;',
    BATCH,
  ],
  [
    'hardware not checked',
    'src/release.js',
    "(record.verdict === 'PASS' && hardware !== 'MATCH') ||",
    'false ||',
    PURE,
  ],
  [
    'release rows stored as release',
    'src/ledger.js',
    "return c === 'release' ? { profile: 'check', budgetClass: 'release' } : { profile: c };",
    'return { profile: c };',
    BATCH,
  ],
  [
    'release takes every slot',
    'src/policy.js',
    'ceiling = release ? cfg.maxConcurrent - 1 : cfg.maxConcurrent,',
    'ceiling = cfg.maxConcurrent,',
    ['edits2-concurrency'],
  ],
  [
    'release writes the commit gap',
    'src/ledger.js',
    "    if (budgetClass(record) !== 'release')\n",
    '',
    BATCH,
  ],
  [
    'contract price skips the cost cap',
    'src/index.js',
    "if (budgetClass(record) === 'release' && typeof record.batch === 'string') {",
    'if (false) {',
    BATCH,
  ],
  [
    'no account daily cap',
    'src/release.js',
    "if (budget.daily_usd + estimate > cfg.accountDaily) refusals.push('ACCOUNT_DAILY_USD');",
    ';',
    PURE,
  ],
  [
    'gap exemption without RUNNING',
    'src/ledger.js',
    "      batch.state === 'RUNNING' &&\n",
    '',
    BATCH,
  ],
  [
    'gap exemption for any leg state',
    'src/ledger.js',
    "batch.legs.some((l) => l.key === key && l.state === 'PENDING')",
    'batch.legs.some((l) => l.key === key)',
    BATCH,
  ],
  [
    'no start window',
    'src/index.js',
    '        now() - (l.queued ?? batch.created) > cfg.releaseStartWindow\n',
    '        false\n',
    BATCH,
  ],
  [
    're-run on the sibling machine',
    'src/index.js',
    '              ...batch.legs.filter((x) => x.die === l.die && x.machineId).map((x) => x.machineId),\n',
    '',
    BATCH,
  ],
  [
    'no account cap in admission',
    'src/ledger.js',
    'if (summary.daily_usd + cost > cfg.accountDaily)',
    'if (false)',
    BATCH,
  ],
  [
    'batches never pruned',
    'src/index.js',
    '        await ledger.storage.delete(key);\n        pruned++;',
    '        pruned++;',
    BATCH,
  ],
);
// Review of PR #269, R3 (2026-11-05): boolean vars and teardown under a malformed configuration.
MUTANTS.push(
  [
    'config error stops teardown',
    'src/index.js',
    '            try {\n              cfg = teardownConfiguration(env);\n            } catch {\n              throw e;\n            }',
    '            throw e;',
    CONFIG,
  ],
  [
    'teardown-only runs batch ticks',
    'src/index.js',
    '          if (teardownOnly) return;\n',
    '',
    CONFIG,
  ],
  [
    'teardown keeps policy vars',
    'src/policy.js',
    'return configuration(identity);',
    'return configuration(env);',
    CONFIG,
  ],
  [
    'Worker boolean unchecked',
    'src/policy.js',
    "    if (env[name] !== undefined && env[name] !== 'true' && env[name] !== 'false')",
    '    if (false)',
    CONFIG,
  ],
  [
    'deploy boolean unchecked',
    'deploy.mjs',
    "    if (vars.has(key) && !['true', 'false'].includes(vars.get(key)))",
    '    if (false)',
    CONFIG,
  ],
);
// Review of PR #269, R4 (2026-11-05): the tier from the verified ref and base branch.
MUTANTS.push(
  [
    'base branch ignored',
    'src/tier.js',
    '    baseRef: branch(c?.base_ref) ?? fromRef,',
    '    baseRef: fromRef,',
    CLAIMS,
  ],
  [
    'base from ref ignored',
    'src/tier.js',
    '    baseRef: branch(c?.base_ref) ?? fromRef,',
    '    baseRef: branch(c?.base_ref),',
    CLAIMS,
  ],
  [
    'tag ref dropped',
    'src/tier.js',
    '    ref,\n    baseRef',
    '    ref: undefined,\n    baseRef',
    CLAIMS,
  ],
  [
    'admission passes only the event',
    'src/index.js',
    '      tierName: tierForClaims(c, profile),',
    '      tierName: tierForClaims({ event_name: c.event_name }, profile),',
    CLAIMS,
  ],
);
const selected = process.argv.slice(2);
let failed = false;
// TAP explicitly: the default reporter differs between a terminal and CI. A kill is a `not ok` line
// that names a test, never a test file that failed to load (review F9).
const run = (copy, tests) => {
  const result = spawnSync(
    process.execPath,
    [
      '--no-warnings',
      '--test',
      '--test-reporter=tap',
      ...tests.map((t) => join(copy, 'test', t + '.test.js')),
    ],
    { cwd: copy, encoding: 'utf8', timeout: 300000, maxBuffer: 32 * 1024 * 1024 },
  );
  const kills = [...(result.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)]
    .map((m) => m[1])
    .filter((t) => !/\.test\.js$/.test(t) && !t.startsWith('/'));
  return { result, kills };
};
const prepare = () => {
  const root = mkdtempSync(join(tmpdir(), 'kf-tier-mutant-'));
  const copy = join(root, 'ci/vast-broker');
  cpSync(broker, copy, { recursive: true, filter: (p) => !p.includes('node_modules') });
  symlinkSync(join(broker, 'node_modules'), join(copy, 'node_modules'));
  // ci/platform-api/migrations: test/d1.js (FakeD1) applies them; batch.test.js's release + store
  // test (integration of #258 with #259, 2026-11-05) needs them; ci/requirements.toml: its
  // release + requirements test (review of PR #269, R2).
  for (const extra of [
    'scripts/ci/vast_report.py',
    'ci/vast-reaper',
    'ci/platform-api/migrations',
    'ci/requirements.toml',
  ]) {
    mkdirSync(join(root, extra, '..'), { recursive: true });
    cpSync(join(repo, extra), join(root, extra), { recursive: true });
  }
  return { root, copy };
};
// Baseline: the unmutated copy must pass every test file the mutants use, so a harness that cannot
// load or run the tests can never report kills.
{
  const { root, copy } = prepare();
  try {
    const files = [...new Set(MUTANTS.flatMap((m) => m[4]))];
    const { result, kills } = run(copy, files);
    const ok = result.status === 0 && !result.error && kills.length === 0;
    console.log(JSON.stringify({ baseline: files, status: result.status, ok }));
    if (!ok) {
      console.log('BASELINE FAILED: no mutant result is meaningful');
      process.exit(1);
    }
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
for (const [name, file, before, after, tests] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const { root, copy } = prepare();
  try {
    const target = join(copy, file);
    const original = readFileSync(target, 'utf8');
    if (original.split(before).length !== 2) throw Error('ambiguous or missing mutation: ' + name);
    writeFileSync(target, original.replace(before, after));
    const { result, kills } = run(copy, tests);
    const ok = result.status !== 0 && kills.length >= 1 && !result.error;
    failed ||= !ok;
    console.log(JSON.stringify({ mutant: name, status: result.status, kills, ok }));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
if (failed) {
  console.log('SURVIVING MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
// The opt-in Windows apps proposal requires two named killers per new rule (J1, 2026-11-07).
if (!failed && selected.length === 0) await import('../winapps-mutants.mjs');
