#!/usr/bin/env node
// GPU-free, isolated mutations of the RAM-by-VRAM offer rule (2026-11-20, test/offer-ram-vram.test.js): the rule itself, which
// legs it binds (the kfml stage list, the tier, --only/--skip), where it is applied (offer policy, release plan, rent path),
// the fraction and the units, the tie to the kfml workload's numbers, and the named reason. Each mutant must be killed by at
// least two named tests; the source tree is never touched (a throwaway copy is mutated).
import {
  cpSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
  symlinkSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const repo = fileURLToPath(new URL('../../', import.meta.url));
const broker = fileURLToPath(new URL('.', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-offer-ram-mutants-'));
const P = 'ci/vast-broker/src/policy.js',
  B = 'ci/vast-broker/src/bandwidth.js',
  L = 'ci/vast-broker/src/release.js',
  I = 'ci/vast-broker/src/index.js',
  R = 'ci/vast-broker/runner.py',
  W = 'scripts/kfml/kfml/workloads/managed_oversub.py';
const cases = [
  // The constraint itself.
  [
    'constraint dropped for kfml legs',
    P,
    "export const kfmlOversubRuns = (leg) =>\n  Array.isArray(leg?.stages) &&\n  leg.stages.includes('kfml') &&\n  KFML_OVERSUB_TIERS.includes(leg.tier);",
    'export const kfmlOversubRuns = (leg) => false;',
  ],
  [
    'constraint never refuses (the admission test is always true)',
    P,
    'atLeast(offer.gpu_ram, 1) &&',
    'true ||',
  ],
  // Which legs are subject: every leg, no stage list, no tier, the payload's --only/--skip.
  [
    'rule applied to every leg',
    P,
    "export const kfmlOversubRuns = (leg) =>\n  Array.isArray(leg?.stages) &&\n  leg.stages.includes('kfml') &&\n  KFML_OVERSUB_TIERS.includes(leg.tier);",
    'export const kfmlOversubRuns = (leg) => true;',
  ],
  [
    'stage list ignored (the tier alone decides)',
    P,
    "Array.isArray(leg?.stages) &&\n  leg.stages.includes('kfml') &&\n  KFML_OVERSUB_TIERS",
    'KFML_OVERSUB_TIERS',
  ],
  [
    'tier ignored (every leg that plans kfml, the middle tier too)',
    P,
    "leg.stages.includes('kfml') &&\n  KFML_OVERSUB_TIERS.includes(leg.tier);",
    "leg.stages.includes('kfml');",
  ],
  [
    'skip mishandled (a leg that skips kfml is still refused)',
    B,
    ['&& !skip.includes(n));', "(!skip.includes('kfml') || only.includes('kfml'))"],
    [');', 'true'],
  ],
  [
    'skip mishandled in the selection only (the plan still honours it)',
    B,
    '&& !skip.includes(n));',
    ');',
  ],
  [
    'only mishandled (a leg that runs other stages only is still refused)',
    B,
    '(!only.length || only.includes(n)) &&',
    '',
  ],
  [
    'stage list not taken from the plan (every tier plans kfml)',
    B,
    'selectStages(plannedStages({ tier }, options), options);',
    "selectStages([...plannedStages({ tier }, options), 'kfml'], options);",
  ],
  // The wiring: the release plan, the rehearsal plan and the rent path apply it to the right leg.
  [
    'release and rehearsal plans ignore the rule',
    L,
    'return offers.filter((o) => !refusedByRamRule(o, leg));',
    'return offers;',
  ],
  ['release plan applies it to middle legs', L, 'legTierOfRow(matrixRow),\n  );', "'large',\n  );"],
  ['rent path names no leg', I, 'leg: { tier: legTier, stages: legStages(legTier) },', ''],
  [
    'rent path rents every leg as large',
    I,
    "const legTier = middleLeg ? 'middle' : tierName;",
    "const legTier = 'large';",
  ],
  // The fraction of the listed RAM a KVM box really gets.
  [
    'KVM fraction taken as the whole listing',
    P,
    'export const KVM_RAM_FRACTION = 0.75;',
    'export const KVM_RAM_FRACTION = 1;',
  ],
  [
    'KVM fraction at the generous end',
    P,
    'export const KVM_RAM_FRACTION = 0.75;',
    'export const KVM_RAM_FRACTION = 0.83;',
  ],
  [
    'KVM fraction below the measured range',
    P,
    'export const KVM_RAM_FRACTION = 0.75;',
    'export const KVM_RAM_FRACTION = 0.5;',
  ],
  // The unit of the offer's VRAM and RAM (Vast: MiB).
  [
    'gpu_ram taken as GB',
    P,
    '(KFML_OVERSUB_FACTOR * gpuRamMb) /',
    '(KFML_OVERSUB_FACTOR * gpuRamMb * 1024) /',
  ],
  [
    'gpu_ram MiB taken as decimal MB',
    P,
    '(KFML_OVERSUB_FACTOR * gpuRamMb) /',
    '(KFML_OVERSUB_FACTOR * gpuRamMb * 1.048576) /',
  ],
  [
    'cpu_ram taken as decimal MB',
    P,
    'atLeast(offer.cpu_ram, listedRamNeededMb(offer.gpu_ram) - LISTED_RAM_TOLERANCE_MB)',
    'atLeast(offer.cpu_ram, listedRamNeededMb(offer.gpu_ram) * 1.048576 - LISTED_RAM_TOLERANCE_MB)',
  ],
  // The numbers of the kfml rule, on both sides of the tie.
  [
    'policy factor drifts',
    P,
    'export const KFML_OVERSUB_FACTOR = 1.25;',
    'export const KFML_OVERSUB_FACTOR = 1.5;',
  ],
  [
    'policy host fraction drifts',
    P,
    'export const KFML_OVERSUB_HOST_FRACTION = 0.6;',
    'export const KFML_OVERSUB_HOST_FRACTION = 0.5;',
  ],
  ['workload factor drifts', W, 'OVERSUB_FACTOR = 1.25', 'OVERSUB_FACTOR = 1.5'],
  [
    'workload host fraction drifts',
    W,
    'OVERSUB_HOST_RAM_FRACTION = 0.6',
    'OVERSUB_HOST_RAM_FRACTION = 0.5',
  ],
  ['workload retypes the factor', W, 'int(total * OVERSUB_FACTOR)', 'int(total * 1.25)'],
  [
    'workload retypes the host fraction',
    W,
    '_host_avail_gb() * OVERSUB_HOST_RAM_FRACTION',
    '_host_avail_gb() * 0.6',
  ],
  // The named reason.
  ['reason missing', P, "reasons.push('RAM_BELOW_VRAM_RULE');", 'void 0;'],
  [
    'reason renamed to the flat floor',
    P,
    "reasons.push('RAM_BELOW_VRAM_RULE');",
    "reasons.push('CPU_RAM');",
  ],
  ['runner does not know the reason', R, '        "RAM_BELOW_VRAM_RULE",\n', ''],
];
function run() {
  const result = spawnSync(
    process.execPath,
    ['--test', '--test-reporter=tap', 'ci/vast-broker/test/offer-ram-vram.test.js'],
    { cwd: scratch, encoding: 'utf8', timeout: 120000 },
  );
  if (result.error) throw result.error;
  const killers = [...result.stdout.matchAll(/^not ok \d+ - (.+)$/gm)]
    .map((m) => m[1])
    .filter((n) => !n.endsWith('.test.js'));
  if (result.status !== 0 && !killers.length) throw Error(result.stderr + result.stdout);
  return [...new Set(killers)];
}
try {
  cpSync(broker, join(scratch, 'ci/vast-broker'), {
    recursive: true,
    filter: (p) => !p.includes('node_modules') && !p.includes('/.wrangler') && !p.includes('/dist'),
  });
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  for (const path of ['scripts/ci/vast_report.py', W]) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path));
  }
  const baseline = run();
  if (baseline.length) throw Error('baseline red: ' + JSON.stringify(baseline));
  const rows = [];
  for (const [name, path, before, after] of cases) {
    const file = join(scratch, path),
      source = readFileSync(file, 'utf8');
    // `before`/`after` may be lists: several edits of one file make one mutant.
    const edits = Array.isArray(before) ? before.map((b, i) => [b, after[i]]) : [[before, after]];
    let changed = source;
    for (const [b, a] of edits) {
      if (changed.split(b).length !== 2) throw Error('mutant anchor: ' + name);
      changed = changed.replace(b, () => a);
    }
    writeFileSync(file, changed);
    let killers;
    try {
      killers = run();
    } finally {
      writeFileSync(file, source);
    }
    if (killers.length < 2)
      throw Error(name + ': fewer than two killers ' + JSON.stringify(killers));
    rows.push({ mutant: name, kills: killers.length, killers });
  }
  console.log(JSON.stringify({ status: 'PASS', cases: rows }, null, 2));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
