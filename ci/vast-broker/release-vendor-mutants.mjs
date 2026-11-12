#!/usr/bin/env node
// CPU-vendor and middle-profile mutants (2026-11-12). Isolated GPU-free mutants. Falsifier: a mutant survives or has fewer than two named killers.
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  copyFileSync,
  rmSync,
  symlinkSync,
  cpSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const here = fileURLToPath(new URL('.', import.meta.url));
const root = fileURLToPath(new URL('../../', import.meta.url));
const V = 'ci/vast-broker/src/release-matrix.js',
  REL = 'ci/vast-broker/src/release.js',
  IDX = 'ci/vast-broker/src/index.js',
  P = 'scripts/ci/vast_payload.py',
  R = 'scripts/ci/vast_report.py';
const cases = [
  // planner: the vendor cover
  [
    'vendor swap never runs',
    REL,
    'guard < 16 && deficit(count()).length',
    'guard < 0 && deficit(count()).length',
  ],
  [
    'swap takes the dearest offer',
    REL,
    'delta < best.delta - 1e-9 ||',
    'delta > best.delta + 1e-9 ||',
  ],
  [
    'swap may trade one vendor for the other',
    REL,
    'if (lack(after) >= lack(cur)) continue;',
    'if (lack(after) > lack(cur)) continue;',
  ],
  [
    'middle legs count for the vendor rule',
    REL,
    "const largeCells = () => families.flatMap((f) => f.cells).filter((c) => c.leg_tier === 'large');",
    'const largeCells = () => families.flatMap((f) => f.cells);',
  ],
  [
    'missing vendor not refused',
    REL,
    'if (vendor.missing.length) {\n    refusals.push',
    'if (false) {\n    refusals.push',
  ],
  [
    'missing vendor not INCOMPLETE',
    REL,
    "if (!refusals.includes('INCOMPLETE')) refusals.push('INCOMPLETE');",
    '',
  ],
  ['covering legs not pinned', REL, 'l.pin_cpu_vendor = v;', ''],
  [
    'unrecognised hosts planned',
    REL,
    'return swappable.filter((o) => rentableCpuName(o.cpu_name));',
    'return swappable;',
  ],
  [
    'unfamiliar CPU names rentable',
    V,
    "['amd', 'intel'].includes(vendorOfCpuName(name))",
    "['amd', 'intel', 'other'].includes(vendorOfCpuName(name))",
  ],
  // planner: the middle profile
  [
    'middle die planned as a large run',
    REL,
    "if (legTier === 'middle') return [leg(first, 1)];",
    '',
  ],
  [
    'middle priced at the large limit',
    REL,
    '(o.pricing.computeStorage * (cfg.releaseMiddleHard + cfg.cronPeriod)) / 3600000',
    '(o.pricing.computeStorage * (cfg.hardRelease + cfg.cronPeriod)) / 3600000',
  ],
  // matrix validation
  [
    'profile per role not validated',
    V,
    "row.profile === (row.role === 'flagship' ? p.full_profile : p.second_die_profile)",
    "['full', 'middle'].includes(row.profile)",
  ],
  [
    'vendor rule shape not validated',
    V,
    'keys(p.cpu_vendor_min, CPU_VENDOR_REQUIRED) &&',
    'true &&',
  ],
  // judge
  ['judge ignores the vendor rule', REL, '          vendor.missing.length === 0 &&\n', ''],
  [
    'judge trusts the offer without the box report',
    REL,
    'else if (offered !== reported) mismatched.push(l.key);',
    'else if (false) mismatched.push(l.key);',
  ],
  [
    'judge counts non-passing legs',
    REL,
    "if (l.legTier === 'middle' || l.outcome?.verdict !== 'PASS') continue;",
    "if (l.legTier === 'middle') continue;",
  ],
  [
    'judge: middle die needs the large Windows count',
    REL,
    "const middleDie = legs.every((l) => l.legTier === 'middle');",
    'const middleDie = false;',
  ],
  [
    'judge: family shape ignored',
    REL,
    '(middleFamilies.size === 0 || (largeFamilies.has(f) && middleFamilies.has(f)))',
    'true',
  ],
  // rental and ingest path
  [
    'middle leg rents the large hard limit',
    IDX,
    'cfg = { ...cfg, hard: cfg.releaseMiddleHard, ttl: cfg.releaseMiddleHard };',
    '',
  ],
  [
    'vendor pin ignored at rent',
    IDX,
    'return rentableCpuName(o.cpu_name) && (leg?.cpuVendor === undefined || v === leg.cpuVendor);',
    'return rentableCpuName(o.cpu_name);',
  ],
  [
    'unrecognised host rented',
    IDX,
    'return rentableCpuName(o.cpu_name) && (leg?.cpuVendor',
    'return (leg?.cpuVendor',
  ],
  [
    'pin not carried into the batch leg',
    IDX,
    '...(selected.pin_cpu_vendor ? { cpuVendor: selected.pin_cpu_vendor } : {}),',
    '',
  ],
  [
    'offered vendor not recorded',
    IDX,
    "...(profile === 'release' ? { cpuVendorOffered: vendorOfCpuName(offer.cpu_name) } : {}),",
    '',
  ],
  [
    'box vendor report not read',
    IDX,
    'l.cpuVendorReported = r?.result?.host_claims?.cpu_vendor ?? null;',
    'l.cpuVendorReported = null;',
  ],
];
const scratch = mkdtempSync(join(tmpdir(), 'kf-vendor-mutants-'));
const testPython = [
  '-c',
  `import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_release_matrix.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))`,
];
const files = [
  'ci/vast-broker/package.json',
  'ci/vast-broker/defaults.json',
  'ci/vast-broker/release-matrix.json',
  'ci/vast-broker/release-plan.mjs',
  'ci/vast-broker/deploy.mjs',
  'ci/vast-broker/validate-release-matrix.mjs',
  'ci/vast-broker/test/release-matrix.test.js',
  'ci/vast-broker/test/release-cpu-vendor.test.js',
  'ci/vast-broker/test/release-safety.test.js',
  'ci/vast-broker/test/tiers-release.test.js',
  'ci/vast-broker/test/batch.test.js',
  'ci/vast-broker/test/release-fixture.js',
  'ci/vast-broker/test/helpers.js',
  'ci/vast-broker/test/d1.js',
  'ci/requirements.toml',
  'ci/vast-broker/test/fixtures/release-market-synthetic.json',
  'scripts/ci/test_vast_release_matrix.py',
  'scripts/ci/release_driver_pins.py',
  P,
  R,
  'scripts/bench/native_xid_guard.py',
  'scripts/bench/native_fault_names.json',
];
// Copy broker sources because the planner integration tests exercise its real policy and offers.
import { readdirSync } from 'node:fs';
for (const name of readdirSync(join(here, 'src')))
  if (name.endsWith('.js')) files.push('ci/vast-broker/src/' + name);
for (const path of files) {
  mkdirSync(join(scratch, path, '..'), { recursive: true });
  copyFileSync(join(root, path), join(scratch, path));
}
symlinkSync(join(here, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
cpSync(join(root, 'ci/platform-api/migrations'), join(scratch, 'ci/platform-api/migrations'), {
  recursive: true,
});
// Each mutant runs the vendor suite and the suites whose expectations the middle profile changed.
const SUITES = ['release-cpu-vendor', 'tiers-release', 'release-matrix'];
function run(python) {
  const result = spawnSync(
    python ? 'python3' : process.execPath,
    python
      ? testPython
      : [
          '--test',
          '--test-reporter=tap',
          ...SUITES.map((name) => 'ci/vast-broker/test/' + name + '.test.js'),
        ],
    {
      cwd: scratch,
      env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' },
      encoding: 'utf8',
      timeout: 300000,
    },
  );
  if (result.error) throw result.error;
  if (python) {
    if (result.status !== 0) throw Error(result.stderr);
    return JSON.parse(result.stdout.trim()).killers;
  }
  const killers = [...result.stdout.matchAll(/^not ok \d+ - (.+)$/gm)]
    .map((m) => m[1])
    .filter((name) => !name.endsWith('.test.js'));
  if (result.status !== 0 && !killers.length) throw Error(result.stderr + result.stdout);
  return killers;
}
try {
  const baseline = run(false);
  if (baseline.length) throw Error('baseline is red: ' + JSON.stringify(baseline));
  const results = [];
  for (const [name, path, before, after, testName] of cases) {
    const file = join(scratch, path),
      source = readFileSync(file, 'utf8');
    let mutated = source;
    const replacements = Array.isArray(before)
      ? before.map((old, i) => [old, after[i]])
      : [[before, after]];
    for (const [old, replacement] of replacements) {
      if (mutated.split(old).length !== 2) throw Error('mutant anchor not unique: ' + name);
      mutated = mutated.replace(old, replacement);
    }
    writeFileSync(file, mutated);
    let killers;
    try {
      killers = [...new Set(run(false))];
    } finally {
      writeFileSync(file, source);
    }
    results.push({ mutant: name, kills: killers.length, killers });
  }
  // Every mutant is run before the verdict, so one weak test does not hide the others.
  const weak = results.filter((r) => r.kills < 2);
  console.log(
    JSON.stringify({ status: weak.length ? 'FAIL' : 'PASS', weak, cases: results }, null, 2),
  );
  if (weak.length) process.exitCode = 1;
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
