#!/usr/bin/env node
// Isolated GPU-free checks. Falsifier: any new rule has fewer than two named killers.
import {
  cpSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
  symlinkSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
const root = fileURLToPath(new URL('../../', import.meta.url));
const broker = fileURLToPath(new URL('.', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-preferred-mutants-'));
const P = 'src/policy.js',
  I = 'src/index.js',
  L = 'src/release.js';
const cases = [
  [
    'preferred boundary made exclusive',
    P,
    'const enough = (key, value) => Number.isFinite(offer[key]) && offer[key] >= value;',
    'const enough = (key, value) => Number.isFinite(offer[key]) && offer[key] > value;',
  ],
  [
    'preferred metadata unchecked',
    P,
    "refuse('CONFIG_VAR:PREFERRED_HOST_QUALITY', 503);",
    'void 0;',
  ],
  [
    'preferred GPU ratio ignored',
    P,
    'ratio >= Math.max(floor.medianRatio, preferred.preferredMedianRatio)',
    'true',
  ],
  [
    'preferred CPU ignored',
    P,
    "enough('cpu_cores_effective', Math.max(floor.cores, preferred.preferredCpuCores))",
    'true',
  ],
  [
    'preferred disk ignored',
    P,
    "enough('disk_bw', Math.max(floor.diskMbps, preferred.preferredDiskMbps))",
    'true',
  ],
  [
    'preferred download ignored',
    P,
    "enough('inet_down', Math.max(floor.downMbps, preferred.preferredDownMbps))",
    'true',
  ],
  ['median ratio fabricated', P, 'const ratio = offer.dlperf / median;', 'const ratio = 1;'],
  ['preferred tier ignored', P, "offer.offer_facts?.performance_tier === 'preferred' ? 0 : 1", '1'],
  [
    'daily affordability ignored',
    P,
    'caps.dailyUsd - (budget?.profiles?.[profile]?.daily_usd ?? 0),',
    'Infinity,',
  ],
  ['account affordability ignored', P, 'cfg.accountDaily - (budget?.daily_usd ?? 0),', 'Infinity,'],
  ['rolling affordability ignored', P, 'cfg.rolling - (budget?.rolling_usd ?? 0),', 'Infinity,'],
  [
    'credit affordability ignored',
    P,
    '...(credit === undefined ? [] : [credit - cfg.reserve - (budget?.outstanding_usd ?? 0)]),',
    '...[],',
  ],
  [
    'affordable fallback ignored',
    I,
    'if (affordable.length) evaluation.allowed = affordable;',
    'void affordable;',
  ],
  [
    'release cap fallback ignored',
    L,
    'dieLegs(affordable.length ? affordable : offers, cfg, matrixRow)',
    'dieLegs(offers, cfg, matrixRow)',
  ],
  [
    'pending leg reservations ignored',
    I,
    'spent + pendingReservation + cost > batch.costCapUsd',
    'spent + cost > batch.costCapUsd',
  ],
  [
    'ledger offer facts lost',
    I,
    'offerFacts: structuredClone(offer.offer_facts ?? null),',
    'offerFacts: null,',
  ],
  [
    'settled snapshot lost',
    'src/ledger.js',
    'offerFacts: structuredClone(record.offerFacts ?? null),',
    'offerFacts: null,',
  ],
  [
    'publisher snapshot lost',
    'src/publish.js',
    'offerFacts: entry.offerFacts ?? null,',
    'offerFacts: null,',
    'publish',
  ],
  ['report facts lost', 'src/runreport.js', 'offer_facts: offerFacts,', 'offer_facts: null,'],
];
function run(mode) {
  const args =
    mode === 'publish'
      ? [
          '--test',
          '--test-reporter=tap',
          '--test-name-pattern',
          'preferred offer facts publish',
          'ci/vast-broker/test/run-report.test.js',
        ]
      : ['--test', '--test-reporter=tap', 'ci/vast-broker/test/preferred-hosts.test.js'];
  const r = spawnSync(process.execPath, args, { cwd: scratch, encoding: 'utf8', timeout: 300000 });
  if (r.error) throw r.error;
  const killers = [...r.stdout.matchAll(/^not ok \d+ - (.+)$/gm)]
    .map((m) => m[1])
    .filter((s) => !s.endsWith('.test.js'));
  if (r.status !== 0 && !killers.length) throw Error(r.stdout + r.stderr);
  return [...new Set(killers)];
}
try {
  for (const relative of ['ci/vast-broker', 'ci/platform-api']) {
    cpSync(join(root, relative), join(scratch, relative), {
      recursive: true,
      filter: (p) =>
        !p.includes('/node_modules') && !p.includes('/.wrangler') && !p.includes('/dist'),
    });
  }
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  for (const relative of ['scripts/ci/vast_report.py', 'ci/requirements.toml']) {
    mkdirSync(dirname(join(scratch, relative)), { recursive: true });
    cpSync(join(root, relative), join(scratch, relative));
  }
  for (const mode of ['unit', 'publish']) {
    const killers = run(mode);
    if (killers.length) throw Error('baseline red ' + JSON.stringify(killers));
  }
  const rows = [];
  for (const [name, relative, before, after, mode] of cases.filter(
    (c) => process.argv.length === 2 || process.argv.slice(2).includes(c[0]),
  )) {
    const file = join(scratch, 'ci/vast-broker', relative),
      source = readFileSync(file, 'utf8');
    if (source.split(before).length !== 2) throw Error('mutant anchor: ' + name);
    writeFileSync(file, source.replace(before, after));
    let killers;
    try {
      killers = run(mode);
    } finally {
      writeFileSync(file, source);
    }
    if (killers.length < 2)
      throw Error(name + ': fewer than two named killers ' + JSON.stringify(killers));
    rows.push({ mutant: name, kills: killers.length, killers });
  }
  console.log(JSON.stringify({ status: 'PASS', cases: rows }, null, 2));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
