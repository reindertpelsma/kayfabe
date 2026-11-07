#!/usr/bin/env node
// Isolated GPU-free mutants. Falsifier: a mutant survives or has fewer than two named killers.
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
  P = 'scripts/ci/vast_payload.py',
  R = 'scripts/ci/vast_report.py';
const cases = [
  [
    'driver environment dropped at rental',
    'ci/vast-broker/src/index.js',
    '          releaseMatrix: leg?.releaseMatrix,',
    '',
    'batch',
  ],
  [
    'retry matrix dropped',
    'ci/vast-broker/src/index.js',
    '          ...(l.releaseMatrix ? { releaseMatrix: l.releaseMatrix } : {}),',
    '',
    'batch',
  ],
  ['subruns dropped from transport', V, 'subruns: row.subruns', 'subruns: []'],
  [
    'eight-box plan expanded to extra legs',
    'ci/vast-broker/src/release.js',
    'releaseMatrix.policy.boxes_per_die > 1',
    'releaseMatrix.policy.boxes_per_die >= 1',
  ],
  [
    'missing die accepted',
    V,
    '  return errors;\n}\nexport function hostProvidesDrivers',
    '  return errors.filter(reason => !reason.includes("count"));\n}\nexport function hostProvidesDrivers',
  ],
  [
    'driver family mismatch accepted',
    V,
    "driverVersion(driver) && driver.split('.')[0] === run.driver_family,",
    'driverVersion(driver),',
  ],
  ['extra driver schema field accepted', V, 'keys(drivers, AXES)', 'object(drivers)'],

  ['duplicate combinations admitted', V, 'ids.every((id) => !combinations.has(id)),', 'true,'],
  ['minimum driver floor ignored', V, 'bad(compare(drivers[axis], floor) >= 0,', 'bad(true,'],
  [
    'unlisted die admitted',
    V,
    'bad(cfg.releaseDies[row.die_family]?.includes(label),',
    'bad(true,',
  ],
  [
    'subrun profiles ignored',
    V,
    [
      'bad(p.subrun_profiles.includes(run.profile),',
      "run.profile === (run.driver_family === fullMajor ? 'small' : 'middle'),",
    ],
    ['bad(true,', 'true,'],
  ],
  [
    'cross driver mismatch ignored',
    V,
    'AXES.every((a) => cross.drivers?.[a] === flagship.drivers?.[a])',
    'true',
  ],
  [
    'fixed host cross requirement ignored',
    V,
    '...(row.cross_subrun ? [row.cross_subrun.drivers] : [])',
    '...[]',
  ],
  [
    'payload proposed execution allowed',
    P,
    '        raise EvidenceError("release_retry_matrix_proposed")',
    '        return None',
  ],
  [
    'host installer mapping dropped',
    P,
    '    selected["HOST_DRIVER"] = selected["KF_HOST_DRIVER"]',
    '    selected["HOST_DRIVER"] = "580.159.04"',
  ],
  [
    'reporter host driver lost',
    R,
    '    ("KF_HOST_DRIVER", False, _match(r"[0-9]{3}\\.[0-9]{2,3}(?:\\.[0-9]{2})?")),\n',
    '',
  ],
];
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-mutants-'));
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
  'ci/vast-broker/validate-release-matrix.mjs',
  'ci/vast-broker/test/release-matrix.test.js',
  'ci/vast-broker/test/batch.test.js',
  'ci/vast-broker/test/release-fixture.js',
  'ci/vast-broker/test/helpers.js',
  'ci/vast-broker/test/d1.js',
  'ci/requirements.toml',
  'ci/vast-broker/test/fixtures/release-market-synthetic.json',
  'scripts/ci/test_vast_release_matrix.py',
  P,
  R,
  'scripts/bench/native_xid_guard.py',
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
function run(python, testName = 'release-matrix') {
  const result = spawnSync(
    python ? 'python3' : process.execPath,
    python
      ? testPython
      : ['--test', '--test-reporter=tap', 'ci/vast-broker/test/' + testName + '.test.js'],
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
  for (const python of [false, true]) {
    const killers = run(python);
    if (killers.length) throw Error('baseline is red: ' + JSON.stringify({ python, killers }));
  }
  if (run(false, 'batch').length) throw Error('batch baseline is red');
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
      killers = [...new Set(run(path.endsWith('.py'), testName))];
    } finally {
      writeFileSync(file, source);
    }
    if (killers.length < 2)
      throw Error(name + ': fewer than two killers: ' + JSON.stringify(killers));
    results.push({ mutant: name, kills: killers.length, killers });
  }
  console.log(JSON.stringify({ status: 'PASS', cases: results }, null, 2));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
