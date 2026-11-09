// GPU-free Windows apps proposal mutations. Two distinct named test failures are required each.
// The fixture uses synthetic tokens and local mocks; no Worker/provider call or deployment.
import {
  cpSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const broker = fileURLToPath(new URL('.', import.meta.url));
const repo = fileURLToPath(new URL('../..', import.meta.url));
const cases = [
  [
    'winapps default enabled',
    'src/tier.js',
    "  const env = { KF_MERGE_WINDOWS: tier(profile).windows ? '1' : '0' };",
    "  winApps ??= 'small';\n  const env = { KF_MERGE_WINDOWS: tier(profile).windows ? '1' : '0' };",
  ],
  ['winapps request dropped', 'src/tier.js', 'env.KF_WINAPPS = winApps;', 'void winApps;'],
  [
    'winapps unknown profile accepted',
    'src/tier.js',
    "!tier(profile).windows || !['small', 'mid', 'full'].includes(winApps)",
    '!tier(profile).windows',
  ],
  [
    'winapps on Linux-only sweep',
    'src/tier.js',
    "!tier(profile).windows || !['small', 'mid', 'full'].includes(winApps)",
    "!['small', 'mid', 'full'].includes(winApps)",
  ],
  [
    'winapps ISO missing',
    'src/benchdata.js',
    "  ['winapps-open.iso', 'winapps-open/kfapps.iso'],\n",
    '',
  ],
  [
    'winapps manifest missing',
    'src/benchdata.js',
    "  ['winapps-open.manifest.json', 'winapps-open/manifest.json'],\n",
    '',
  ],
  [
    'winapps ISO wrong key',
    'src/benchdata.js',
    "['winapps-open.iso', 'winapps-open/kfapps.iso']",
    "['winapps-open.iso', 'winapps-open/wrong.iso']",
  ],
  [
    'winapps manifest wrong key',
    'src/benchdata.js',
    "['winapps-open.manifest.json', 'winapps-open/manifest.json']",
    "['winapps-open.manifest.json', 'winapps-open/wrong.json']",
  ],
];

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'kf-winapps-worker-mutants-'));
  const copy = join(root, 'ci/vast-broker');
  cpSync(broker, copy, {
    recursive: true,
    filter: (p) => !p.includes('node_modules') && !p.includes('.wrangler'),
  });
  symlinkSync(join(broker, 'node_modules'), join(copy, 'node_modules'));
  for (const extra of [
    'scripts/ci/vast_report.py',
    'scripts/ci/release_driver_pins.py',
    'ci/vast-reaper',
    'ci/platform-api/migrations',
    'ci/requirements.toml',
  ]) {
    mkdirSync(join(root, extra, '..'), { recursive: true });
    cpSync(join(repo, extra), join(root, extra), { recursive: true });
  }
  return { root, copy };
}
function run(copy) {
  const result = spawnSync(
    process.execPath,
    [
      '--test',
      '--test-reporter=tap',
      '--test-concurrency=1',
      'test/tier.test.js',
      'test/bench-data.test.js',
    ],
    {
      cwd: copy,
      encoding: 'utf8',
      timeout: 180000,
      maxBuffer: 4 * 1024 * 1024,
    },
  );
  const killers = [
    ...new Set(
      [...(result.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)]
        .map((x) => x[1])
        .filter((x) => !x.endsWith('.test.js') && !x.startsWith('/')),
    ),
  ];
  const tests = Number((result.stdout ?? '').match(/^# tests (\d+)$/m)?.[1] ?? 0);
  return { status: result.status, error: result.error?.code, tests, killers };
}
{
  const { root, copy } = fixture();
  try {
    const baseline = run(copy);
    console.log(JSON.stringify({ baseline: true, ...baseline }));
    if (baseline.status !== 0 || baseline.error || baseline.tests < 1 || baseline.killers.length)
      throw Error('Windows apps baseline red');
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
let failed = false;
for (const [name, file, before, after] of cases) {
  const { root, copy } = fixture();
  try {
    const path = join(copy, file);
    const source = readFileSync(path, 'utf8');
    if (source.split(before).length !== 2) throw Error('mutation anchor: ' + name);
    writeFileSync(path, source.replace(before, after));
    const result = run(copy);
    const ok = result.status !== 0 && !result.error && result.killers.length >= 2;
    console.log(JSON.stringify({ mutant: name, count: result.killers.length, ...result, ok }));
    failed ||= !ok;
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
if (failed) process.exitCode = 1;
