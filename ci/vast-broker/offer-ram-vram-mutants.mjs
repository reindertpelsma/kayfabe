#!/usr/bin/env node
// GPU-free, isolated mutations of the RAM-by-VRAM offer rule (2026-11-20, test/offer-ram-vram.test.js). Each mutant
// must be killed by at least two named tests; the source tree is never touched (a throwaway copy is mutated).
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
  R = 'ci/vast-broker/runner.py',
  W = 'scripts/kfml/kfml/workloads/managed_oversub.py';
const cases = [
  // The constraint itself.
  [
    'constraint dropped',
    P,
    'KFML_OVERSUB_PROFILES.includes(resourceProfileName(cfg, profile)) &&',
    'false &&',
  ],
  [
    'constraint never refuses (the admission test is always true)',
    P,
    "enough('gpu_ram', 1) &&",
    'true ||',
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
  // The unit of the offer's VRAM (Vast: MiB).
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
    "enough('cpu_ram', listedRamNeededMb(offer.gpu_ram) - LISTED_RAM_TOLERANCE_MB)",
    "enough('cpu_ram', listedRamNeededMb(offer.gpu_ram) * 1.048576 - LISTED_RAM_TOLERANCE_MB)",
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
  // The profiles the rule covers.
  [
    'rule also covers the middle tier',
    P,
    "KFML_OVERSUB_PROFILES = ['large'];",
    "KFML_OVERSUB_PROFILES = ['middle', 'large'];",
  ],
  [
    'rule covers every tier',
    P,
    "KFML_OVERSUB_PROFILES = ['large'];",
    "KFML_OVERSUB_PROFILES = ['small', 'middle', 'large', 'adhoc'];",
  ],
  ['rule covers no tier', P, "KFML_OVERSUB_PROFILES = ['large'];", 'KFML_OVERSUB_PROFILES = [];'],
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
    if (source.split(before).length !== 2) throw Error('mutant anchor: ' + name);
    writeFileSync(
      file,
      source.replace(before, () => after),
    );
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
