#!/usr/bin/env node
// Release resilience mutants (2026-11-12). Isolated GPU-free mutants of the rule "a failed rental is replaced on the fly and never means FAIL".
// Falsifier: a mutant survives or has fewer than two named killers, except the one named in ONE below.
// Real killer counts (PR #362 review): three mutants have two killers that are ONE test with two parameters ('the release cost cap is not
// applied to a rental': 0 and 2 USD room; 'the replacement may rent the machine that failed' and 'lane B rents the machine its lane A failed
// on': exited and absent). 'an unproven leg is not stored before the next leg rents' has ONE killer (the ghost-create release test). Every
// other mutant has two or more independent tests.
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
  R = 'scripts/ci/vast_report.py',
  POLICY = 'ci/vast-broker/src/policy.js';
const cases = [
  [
    'a failed leg is never re-run',
    REL,
    '(leg.rerunDepth ?? 0) < reruns',
    '(leg.rerunDepth ?? 0) < 0',
  ],
  [
    'the re-run allowance is exceeded by one',
    REL,
    '(leg.rerunDepth ?? 0) < reruns',
    '(leg.rerunDepth ?? 0) < reruns + 1',
  ],
  [
    'a guest FAIL is re-run',
    REL,
    "['RETRY', 'UNATTRIBUTED'].includes(leg.outcome.verdict) &&",
    "['RETRY', 'UNATTRIBUTED', 'FAIL'].includes(leg.outcome.verdict) &&",
  ],
  ['an owner stop is re-run', REL, "    leg.outcome.reason !== 'STOPPED' &&\n", ''],
  [
    'a rental or box loss is a FAIL',
    REL,
    "if (record.verdict !== 'FAIL' || !result) return finish(all('RETRY'), record.verdict ?? 'NONE');",
    "if (record.verdict !== 'FAIL' || !result) return finish(all('FAIL'), record.verdict ?? 'NONE');",
  ],
  [
    'an owner stop is a FAIL',
    REL,
    "return finish(all('UNATTRIBUTED'), 'STOPPED');",
    "return finish(all('FAIL'), 'STOPPED');",
  ],
  [
    'the release passes with a vendor uncovered',
    REL,
    'complete &&\n          vendor.missing.length === 0 &&',
    'complete &&\n          true &&',
  ],
  [
    'the replacement may rent the machine that failed',
    IDX,
    '...batch.legs.filter((x) => x.die === l.die && x.machineId).map((x) => x.machineId),',
    '',
  ],
  ['the replacement drops the vendor pin', IDX, '...(pin ? { cpuVendor: pin } : {}),', ''],
  [
    'the replacement keeps only the failed leg pin',
    REL,
    'return vendorCover(batch, v, leg).length < min[v] ? v : undefined;',
    'return undefined;',
  ],
  [
    'a vanished vendor pin is never given away',
    IDX,
    'movePin(batch, l, RELEASE_MATRIX.policy.cpu_vendor_min);',
    'null;',
  ],
  [
    'a given-away pin lands on a middle leg',
    REL,
    "      l.state === 'PENDING' &&\n      l.legTier !== 'middle' &&\n      !l.cpuVendor &&",
    "      l.state === 'PENDING' &&\n      !l.cpuVendor &&",
  ],
  [
    'an unproven create holds up the legs after it',
    IDX,
    "          await ledger.put('batch:' + batch.id, batch);\n          continue;",
    "          await ledger.put('batch:' + batch.id, batch);\n          break;",
  ],
  [
    'an unproven leg is not stored before the next leg rents',
    IDX,
    "          await ledger.put('batch:' + batch.id, batch);\n          continue;",
    '          continue;',
  ],
  [
    'lane B rents the machine its lane A failed on',
    IDX,
    "if (['RETRY', 'UNATTRIBUTED'].includes(dependency.outcome?.verdict)) {",
    'if (false) {',
  ],
  [
    'lane B does not wait for lane A',
    IDX,
    "        if (dependency.state !== 'ENDED') continue;",
    '',
  ],
  [
    'the release may take every account slot',
    POLICY,
    [
      'ceiling = release ? cfg.maxConcurrent - 1 : cfg.maxConcurrent,',
      'fallback = release ? Math.min(DEFAULTS.maxConcurrentRelease, ceiling) : ceiling,',
    ],
    ['ceiling = cfg.maxConcurrent,', 'fallback = ceiling,'],
  ],
  [
    'the release cost cap is not applied to a rental',
    IDX,
    'if (spent + cost > batch.costCapUsd)',
    'if (false)',
  ],
];
const scratch = mkdtempSync(join(tmpdir(), 'kf-resilience-mutants-'));
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
  'ci/vast-broker/test/release-resilience-fixture.js',
  'ci/vast-broker/test/release-resilience.test.js',
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
// Each mutant runs the resilience suite and the suites that also pin the release verdict, re-run and rent rules.
const SUITES = ['release-resilience', 'release-safety', 'release-cpu-vendor', 'tiers-release'];
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
  const ONE = new Set(['an unproven leg is not stored before the next leg rents']);
  const weak = results.filter((r) => r.kills < (ONE.has(r.mutant) ? 1 : 2));
  console.log(
    JSON.stringify({ status: weak.length ? 'FAIL' : 'PASS', weak, cases: results }, null, 2),
  );
  if (weak.length) process.exitCode = 1;
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
