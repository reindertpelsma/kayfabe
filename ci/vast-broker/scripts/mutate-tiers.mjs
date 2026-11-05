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
    're-run on the failed machine',
    'src/index.js',
    'avoidMachines: [...l.avoidMachines, ...(r?.machineId ? [r.machineId] : [])],',
    'avoidMachines: [...l.avoidMachines],',
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
    "if (record.source === 'admin') return;",
    '',
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

const selected = process.argv.slice(2);
let failed = false;
for (const [name, file, before, after, tests] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const root = mkdtempSync(join(tmpdir(), 'kf-tier-mutant-'));
  const copy = join(root, 'ci/vast-broker');
  try {
    cpSync(broker, copy, { recursive: true, filter: (p) => !p.includes('node_modules') });
    symlinkSync(join(broker, 'node_modules'), join(copy, 'node_modules'));
    // Files the tests read outside the broker directory.
    for (const extra of ['scripts/ci/vast_report.py', 'ci/vast-reaper/wrangler.toml']) {
      mkdirSync(join(root, extra, '..'), { recursive: true });
      cpSync(join(repo, extra), join(root, extra));
    }
    const target = join(copy, file);
    const original = readFileSync(target, 'utf8');
    if (original.split(before).length !== 2) throw Error('ambiguous or missing mutation: ' + name);
    writeFileSync(target, original.replace(before, after));
    const run = spawnSync(
      process.execPath,
      ['--no-warnings', '--test', ...tests.map((t) => join(copy, 'test', t + '.test.js'))],
      { cwd: copy, encoding: 'utf8', timeout: 300000, maxBuffer: 32 * 1024 * 1024 },
    );
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)].map((m) => m[1]);
    const ok = run.status !== 0 && kills.length >= 1 && !run.error;
    failed ||= !ok;
    console.log(JSON.stringify({ mutant: name, status: run.status, kills, ok }));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
if (failed) {
  console.log('SURVIVING MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
