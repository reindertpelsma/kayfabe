#!/usr/bin/env node
// Two independent named invariant probes must kill each simulated renting mutation.
import {
  cpSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  symlinkSync,
  rmSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const broker = fileURLToPath(new URL('.', import.meta.url)),
  repo = fileURLToPath(new URL('../../', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-simulation-mutants-'));
const cases = [
  [
    'candidate bad machine accepted',
    'src/policy.js',
    [['cfg.badMachines.includes(offer.machine_id)', 'false']],
  ],
  [
    'candidate bad host accepted',
    'src/policy.js',
    [['cfg.badHosts.includes(offer.host_id)', 'false']],
  ],
  [
    'configured bad machine reserved',
    'src/ledger.js',
    [['cfg.badMachines.includes(record.machineId)', 'false']],
  ],
  [
    'configured bad host reserved',
    'src/ledger.js',
    [['cfg.badHosts.includes(record.hostId)', 'false']],
  ],
  [
    'persisted bad machine reserved',
    'src/ledger.js',
    [["bad.has('bad-machine:' + record.machineId)", 'false']],
  ],
  [
    'persisted bad host reserved',
    'src/ledger.js',
    [["badHosts.has('bad-host:' + record.hostId)", 'false']],
  ],

  [
    'malformed price fields accepted',
    'src/policy.js',
    [
      ["typeof offer[key] !== 'number' ||", 'false ||'],
      ['!Number.isFinite(offer[key]) ||', 'false ||'],
    ],
  ],

  [
    'per-GB caps ignored',
    'src/policy.js',
    [
      ['offer.inet_up_cost > resources.maxInetUsdPerGb ||', 'false ||'],
      ['offer.inet_down_cost > resources.maxInetUsdPerGb ||', 'false ||'],
    ],
  ],
  [
    'profile hourly cap ignored',
    'src/policy.js',
    [
      ['price.effective > caps.price ||', 'false ||'],
      ['price.total > (caps.price * (cfg.hard + cfg.cronPeriod)) / 3600000', 'false'],
    ],
  ],
  [
    'duplicate machine admitted',
    'src/ledger.js',
    [
      [
        'open.some((o) => Number.isSafeInteger(o.machineId) && o.machineId === record.machineId)',
        'false',
      ],
    ],
  ],
  [
    'atomic concurrency ignored',
    'src/ledger.js',
    [
      ['open.length >= cfg.maxConcurrent ||', 'false ||'],
      [
        'open.filter((o) => budgetClass(o) === budgetClass(record)).length >=\n          cfg.maxConcurrentProfile[budgetClass(record)]',
        'false',
      ],
    ],
  ],
  [
    'release plan cap ignored',
    'src/release.js',
    [
      [
        "if (estimate > cfg.releaseCostCap) refusals.push('COST_CAP');",
        "if (false) refusals.push('COST_CAP');",
      ],
    ],
  ],
  [
    'UNPINNED release admitted',
    'src/release-matrix.js',
    [["value.driver_installers.every((p) => p?.status === 'PINNED')", 'true']],
  ],
  ['offer tie depends on input order', 'src/offers.js', []],
  ['release tie depends on input order', 'src/release.js', [['a.id - b.id;', '0;']]],
  [
    'network quote omitted',
    'src/policy.js',
    [
      [
        'resources.bandwidthDownGb * offer.inet_down_cost + resources.bandwidthUpGb * offer.inet_up_cost',
        '0',
      ],
    ],
  ],
];
function run() {
  const r = spawnSync(
    process.execPath,
    ['--test', '--test-reporter=tap', 'ci/vast-broker/test/simulation-invariants.test.js'],
    {
      cwd: scratch,
      encoding: 'utf8',
      timeout: 300000,
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' },
    },
  );
  if (r.error) throw r.error;
  const names = [...r.stdout.matchAll(/^not ok \d+ - (.+)$/gm)]
    .map((m) => m[1])
    .filter((s) => !s.endsWith('.test.js'));
  if (r.status !== 0 && !names.length) throw Error(r.stdout + r.stderr);
  return [...new Set(names)];
}
try {
  cpSync(broker, join(scratch, 'ci/vast-broker'), {
    recursive: true,
    filter: (p) =>
      !p.includes('/node_modules') && !p.includes('/.wrangler') && !p.includes('/dist'),
  });
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  for (const path of ['scripts/ci/vast_report.py', 'scripts/ci/vast_payload.py']) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path));
  }
  const baseline = run();
  if (baseline.length) throw Error('baseline red ' + JSON.stringify(baseline));
  const rows = [];
  for (const [name, path, replacements] of cases) {
    const file = join(scratch, 'ci/vast-broker', path),
      original = readFileSync(file, 'utf8');
    let changed = original;
    if (name === 'offer tie depends on input order') {
      if (!changed.includes('a.id - b.id')) throw Error('tie anchor');
      changed = changed.replaceAll('a.id - b.id', '0');
    } else
      for (const [before, after] of replacements) {
        if (changed.split(before).length !== 2) throw Error('anchor ' + name);
        changed = changed.replace(before, after);
      }
    writeFileSync(file, changed);
    let killers;
    try {
      killers = run();
    } finally {
      writeFileSync(file, original);
    }
    if (killers.length < 2)
      throw Error(name + ': fewer than two killers ' + JSON.stringify(killers));
    rows.push({ mutant: name, kills: killers.length, killers });
  }
  console.log(JSON.stringify({ status: 'PASS', cases: rows }, null, 2));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
