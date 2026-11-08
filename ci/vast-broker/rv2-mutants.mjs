#!/usr/bin/env node
// GPU-free RV2 regressions: count >=2 independently named invariant failures.
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
const repo = fileURLToPath(new URL('../../', import.meta.url)),
  broker = fileURLToPath(new URL('.', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-rv2-mutants-'));
const P = 'ci/vast-broker/src/policy.js',
  L = 'ci/vast-broker/src/release.js',
  D = 'ci/vast-broker/defaults.json',
  G = 'scripts/ci/profile_resources.py';
const one = (name, file, before, after, mode = 'node', pattern) => ({
  name,
  file,
  before,
  after,
  mode,
  pattern,
});
const cases = [
  one(
    'media helper imports reporter again',
    'scripts/ci/release_driver_media.py',
    'import release_driver_pins',
    'import vast_report as release_driver_pins',
    'python-media',
  ),
  {
    name: 'media drift precedes attribution',
    file: L,
    mode: 'node',
    pattern: 'RV2 drift respects',
    edit: (s) => {
      const begin = s.indexOf(
        '  if (rows.some((r) => r.fail > 0 && /^(?:[a-z0-9]+_)*driver_media_drift$/.test(r.name)))',
      );
      const end = s.indexOf('  const passing =', begin);
      if (begin < 0 || end < 0) throw Error('media anchor');
      const block = s.slice(begin, end);
      s = s.slice(0, begin) + s.slice(end);
      return s.replace(
        "  if (record.closed === 'STOPPED'",
        block + "  if (record.closed === 'STOPPED'",
      );
    },
  },
  one(
    'stopped precedence removed',
    L,
    "if (record.closed === 'STOPPED' && record.verdict !== 'PASS')",
    'if (false)',
    'node',
    'RV2 drift respects stopped',
  ),
  one(
    'hardware precedence removed',
    L,
    "(record.verdict === 'FAIL' && result && hardware === 'MISMATCH')",
    'false',
    'node',
    'RV2 drift respects wrong',
  ),
  one(
    'SPEC row contract ignored',
    P,
    'JSON.stringify(slot.requiredRowsPerDie) !== \'["specviewperf_lanea","specviewperf_laneb"]\' ||',
    'false ||',
    'node',
    'RV2 empty SPEC rows',
  ),
  one(
    'SPEC lane contract ignored',
    P,
    'JSON.stringify(slot.lanes) !== \'["lanea","laneb"]\' ||',
    'false ||',
    'node',
    'RV2 SPEC policy refuses lanes',
  ),
  one(
    'SPEC separate rental contract ignored',
    P,
    'slot.requiresSeparateRentals !== true',
    'false',
    'node',
    'RV2 SPEC separate-rental contract',
  ),
  {
    name: 'SPEC row list missing in data',
    file: D,
    mode: 'node',
    pattern: 'RV2 shipped SPEC contract',
    edit: (s) => {
      const d = JSON.parse(s);
      delete d.stageSlots.specviewperf.requiredRowsPerDie;
      return JSON.stringify(d);
    },
  },
  {
    name: 'SPEC row list empty in data',
    file: D,
    mode: 'node',
    pattern: 'RV2 shipped SPEC contract',
    edit: (s) => {
      const d = JSON.parse(s);
      d.stageSlots.specviewperf.requiredRowsPerDie = [];
      return JSON.stringify(d);
    },
  },
  one(
    'profile cap finiteness omitted',
    P,
    '!finite(base.maxInetUsdPerGb, 1) ||\n    base.maxInetUsdPerGb <= 0',
    'false',
    'node',
    'RV2 invalid middle cap',
  ),
  one(
    'profile inbound typing omitted',
    P,
    '!finite(base.bandwidthDownGb, 10000)',
    'false',
    'node',
    'RV2 profile rejects malformed .*bandwidthDownGb',
  ),
  one(
    'profile outbound typing omitted',
    P,
    '!finite(base.bandwidthUpGb, 10000)',
    'false',
    'node',
    'RV2 profile rejects malformed .*bandwidthUpGb',
  ),
  one(
    'profile disk typing omitted',
    P,
    '!Number.isSafeInteger(base.diskGb) ||\n    base.diskGb <= 0 ||\n    base.diskGb > 10000',
    'false',
    'node',
    'RV2 profile rejects malformed .*diskGb',
  ),
  one(
    'profile keys opened',
    P,
    "Object.keys(base).sort().join(',') !== keys.join(',')",
    'false',
    'node',
    'RV2 resource keys',
  ),
  one(
    'computed price may be infinite',
    P,
    'if (![computeStorage, bandwidth, effective, total].every((n) => Number.isFinite(n) && n >= 0))',
    'if (false)',
    'node',
    'RV2 quote rejects computed overflow',
  ),
  one(
    'Python disk finiteness omitted',
    G,
    'if not profile_numbers_valid(profile):',
    'if False:',
    'python-resources',
  ),
  one(
    'Python SPEC rows ignored',
    G,
    'or spec.get("requiredRowsPerDie")\n        != ["specviewperf_lanea", "specviewperf_laneb"]',
    'or False',
    'python-resources',
  ),
];
function run(mode = 'node', pattern = '.') {
  let args, bin;
  if (mode.startsWith('python')) {
    bin = 'python3';
    const file =
      mode === 'python-media' ? 'test_release_driver_media.py' : 'test_vast_profile_resources.py';
    args = [
      '-c',
      `import io,json,unittest\ns=unittest.defaultTestLoader.discover('scripts/ci',pattern=${JSON.stringify(file)})\nr=unittest.TextTestRunner(stream=io.StringIO()).run(s)\nprint(json.dumps({'tests':r.testsRun,'failures':[t.id() for t,_ in r.failures],'errors':[t.id() for t,_ in r.errors]}))`,
    ];
  } else {
    bin = process.execPath;
    args = [
      '--test',
      '--test-reporter=tap',
      '--test-name-pattern=' + pattern,
      'ci/vast-broker/test/rv2-regressions.test.js',
    ];
  }
  const out = spawnSync(bin, args, {
    cwd: scratch,
    encoding: 'utf8',
    timeout: 120000,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' },
  });
  if (out.error) throw out.error;
  if (mode.startsWith('python')) {
    if (out.status !== 0) throw Error(out.stderr);
    const x = JSON.parse(out.stdout);
    if (x.errors.length) throw Error('unexpected test/setup errors ' + JSON.stringify(x.errors));
    return x.failures;
  }
  const names = [...out.stdout.matchAll(/^not ok \d+ - (RV2 .*)$/gm)].map((m) => m[1]);
  if (out.status !== 0 && !names.length) throw Error(out.stdout + out.stderr);
  return [...new Set(names)];
}
try {
  cpSync(broker, join(scratch, 'ci/vast-broker'), {
    recursive: true,
    filter: (p) =>
      !p.includes('/node_modules') && !p.includes('/.wrangler') && !p.includes('/dist'),
  });
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  cpSync(join(repo, 'scripts/ci'), join(scratch, 'scripts/ci'), {
    recursive: true,
    filter: (p) => !p.includes('__pycache__'),
  });
  const paths = [
    'scripts/bench/native_xid_guard.py',
    'scripts/bench/provision_host_driver.sh',
    'scripts/bench/box/provision_full.sh',
    ...Object.values(
      JSON.parse(readFileSync(join(repo, 'ci/vast-broker/defaults.json'))).payloadStageResources,
    ).map((s) => s.source),
  ];
  for (const path of new Set(paths)) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path));
  }
  for (const mode of ['node', 'python-media', 'python-resources']) {
    const names = run(mode);
    if (names.length) throw Error('baseline failures ' + JSON.stringify(names));
  }
  const result = [];
  for (const c of cases) {
    const path = join(scratch, c.file),
      original = readFileSync(path, 'utf8');
    let mutated;
    if (c.edit) mutated = c.edit(original);
    else {
      if (original.split(c.before).length !== 2) throw Error('ambiguous/missing anchor ' + c.name);
      mutated = original.replace(c.before, c.after);
    }
    writeFileSync(path, mutated);
    let killers;
    try {
      killers = run(c.mode, c.pattern);
    } finally {
      writeFileSync(path, original);
    }
    if (killers.length < 2)
      throw Error(c.name + ' fewer than two intended killers ' + JSON.stringify(killers));
    result.push({ mutant: c.name, kills: killers.length, killers });
  }
  console.log(JSON.stringify({ status: 'PASS', cases: result }, null, 2));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
