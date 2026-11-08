#!/usr/bin/env node
// GPU-free RV1/SPEC safety rules. Every mutation needs two distinct named killers.
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
const repo = fileURLToPath(new URL('../../', import.meta.url));
const broker = fileURLToPath(new URL('.', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-safety-mutants-'));
const I = 'ci/vast-broker/src/index.js',
  V = 'ci/vast-broker/src/release-matrix.js',
  L = 'ci/vast-broker/src/release.js',
  G = 'ci/vast-broker/src/github.js',
  R = 'scripts/ci/vast_report.py';
const cases = [
  [
    'cached legacy release PASS displayed',
    I,
    "batch.tier === 'large' ? releaseVerdict(batch).verdict : batch.verdict",
    'batch.verdict',
    'node',
  ],

  ['merged SPEC environment lost', 'ci/vast-broker/src/tier.js', 'env.KF_SPEC = spec;', '', 'node'],
  [
    'merged Windows apps environment lost',
    'ci/vast-broker/src/tier.js',
    'env.KF_WINAPPS = winApps;',
    '',
    'node',
  ],

  [
    'root executable status ignored',
    V,
    "if (\n    value.status !== 'executable' ||",
    'if (\n    false ||',
    'node',
  ],
  ['die executable status ignored', V, "row.status !== 'executable' ||", 'false ||', 'node'],
  ['recorded blockers ignored', V, 'row.blockers.length !== 0 ||', 'false ||', 'node'],
  [
    'unknown driver minima executable',
    V,
    "Object.values(row.minimum_drivers).some((v) => v === 'UNKNOWN') ||",
    'false ||',
    'node',
  ],
  [
    'unknown Windows ABI executable',
    V,
    "        row.windows_guest_abi === 'UNKNOWN' ||",
    'false ||',
    'node',
  ],

  ['accepted Windows subrun FAIL laundered', L, '', '', 'launder'],
  [
    'proposed trusted policy admitted',
    V,
    "value.status !== 'executable' ||",
    'false ||',
    'proposal',
  ],
  [
    'runtime schema validation omitted',
    V,
    "if (validateReleaseMatrix(value, cfg).length) return 'MATRIX_INVALID';",
    '',
    'node',
  ],
  [
    'persisted selected row not bound',
    I,
    '!expected || !sameMatrixRow(selected, expected)',
    '!expected',
    'node',
  ],
  [
    'receipt reporter capability ignored',
    G,
    'BOX_RELEASE_MATRIX_RECEIPTS',
    'BOX_PROTOCOL_UNUSED_RECEIPTS',
    'reporter-cap',
  ],
  [
    'payload executor capability ignored',
    G,
    'PAYLOAD_RELEASE_MATRIX_EXECUTOR',
    'PAYLOAD_UNUSED_MATRIX_EXECUTOR',
    'payload-cap',
  ],
  [
    'driver receipt binding omitted',
    L,
    '!Object.keys(definition.drivers).every(',
    'false && !Object.keys(definition.drivers).every(',
    'node',
  ],
  [
    'Windows ABI binding omitted',
    L,
    'receipt.windows_guest_abi !== definition.windows_guest_abi ||',
    'false ||',
    'node',
  ],
  [
    'cross receipts omitted',
    V,
    'if (object(row.cross_subrun) && keys(row.cross_subrun.drivers, AXES))',
    'if (false)',
    'node',
  ],
  ['subrun suite coverage ignored', L, "rowValues.every((v) => v === 'PASS')", 'true', 'node'],
  ['matrix proof shape ignored', L, 'if (!checkedMatrixReceipts(proof))', 'if (false)', 'node'],
  [
    'ordinary rows certify matrix classifier',
    L,
    "const matrixRequired = releaseMatrix !== undefined || cfg?.profile === 'release';",
    'const matrixRequired = false;',
    'node',
  ],
  [
    'release reducer omits matrix coverage',
    L,
    "if (batch.tier === 'large') {",
    'if (false) {',
    'node',
  ],
  [
    'result store omits matrix coverage',
    'ci/vast-broker/src/ingest.js',
    "if (verdict === 'PASS' && (profile === 'release' || r.tier === 'large' || r.releaseMatrix))",
    'if (false)',
    'node',
  ],
  [
    'persisted ended rows not regraded',
    I,
    "if (!['STARTED', 'ENDED'].includes(l.state)) continue;",
    "if (l.state !== 'STARTED') continue;",
    'node',
  ],
  [
    'later pass erases cell failure',
    L,
    "values.includes('FAIL')\n      ? 'FAIL'\n      : passes >= required",
    "false\n      ? 'FAIL'\n      : passes >= required",
    'node',
  ],
  [
    'SPEC aggregate row certifies both lanes',
    L,
    "stage === 'specviewperf' ? [...DEFAULTS.stageSlots.specviewperf.requiredRowsPerDie] : [stage]",
    '[stage]',
    'node',
  ],
  [
    'SPEC second rental omitted',
    L,
    'return lanes.map((spec, index) =>',
    'return lanes.slice(0, 1).map((spec, index) =>',
    'node',
  ],
  [
    'SPEC second rental unpriced',
    L,
    'est_usd: round(o.pricing.total),',
    "est_usd: extra.spec === 'laneb' ? 0 : round(o.pricing.total),",
    'node',
  ],
  ['SPEC dependency scheduling omitted', I, 'if (l.waitForLeg) {', 'if (false) {', 'node'],
  [
    'SPEC rental window begins with batch',
    I,
    'if (l.queued === null) l.queued = now();',
    'if (l.queued === null) l.queued = batch.created;',
    'node',
  ],
  [
    'matrix HTTP receipts unsupported',
    'ci/vast-broker/src/reports.js',
    '(matrix ? 6 : 4)',
    '4',
    'node',
  ],
  [
    'reporter drops receipt wire',
    R,
    '"matrix_receipts": matrix_receipts',
    '"matrix_receipts": None',
    'python',
  ],
  ['reporter leaks new wire to old Worker', R, ' and env.get("KF_RELEASE_SUBRUNS")', '', 'python'],
  [
    'reporter omits typed validation',
    R,
    '            validate_matrix_receipts(doc["matrix_receipts"])',
    '            pass',
    'python',
  ],
  [
    'reporter accepts failed proof with PASS',
    R,
    'if doc["verdict"] == "PASS" and any(',
    'if False and any(',
    'python',
  ],
  [
    'reporter cannot transport executable policy',
    R,
    'parsed["status"] in ("proposed", "executable")',
    'parsed["status"] == "proposed"',
    'python',
  ],
];
function run(mode = 'node') {
  const python = mode === 'python';
  const args = python
    ? [
        '-c',
        `import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_matrix_receipts.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'killers':[t.id() for t,_ in r.failures+r.errors]}))`,
      ]
    : ['--test', '--test-reporter=tap', 'ci/vast-broker/test/release-safety.test.js'];
  const r = spawnSync(python ? 'python3' : process.execPath, args, {
    cwd: scratch,
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1' },
    encoding: 'utf8',
    timeout: 300000,
  });
  if (r.error) throw r.error;
  if (python) {
    if (r.status !== 0) throw Error(r.stderr);
    return JSON.parse(r.stdout.trim()).killers;
  }
  const names = [...r.stdout.matchAll(/^not ok \d+ - (.+)$/gm)]
    .map((m) => m[1])
    .filter((n) => !n.endsWith('.test.js'));
  if (r.status !== 0 && !names.length) throw Error(r.stderr + r.stdout);
  return [...new Set(names)];
}
try {
  cpSync(broker, join(scratch, 'ci/vast-broker'), {
    recursive: true,
    filter: (p) => !p.includes('node_modules') && !p.includes('/.wrangler') && !p.includes('/dist'),
  });
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  for (const p of [
    'scripts/ci/vast_report.py',
    'scripts/ci/release_driver_pins.py',
    'scripts/ci/vast_payload.py',
    'scripts/ci/test_vast_matrix_receipts.py',
    'scripts/bench/native_xid_guard.py',
  ]) {
    mkdirSync(dirname(join(scratch, p)), { recursive: true });
    cpSync(join(repo, p), join(scratch, p));
  }
  for (const mode of ['node', 'python']) {
    const failures = run(mode);
    if (failures.length) throw Error('baseline red ' + JSON.stringify(failures));
  }
  const rows = [];
  const selected = process.argv.slice(2);
  for (const [name, path, before, after, mode] of cases) {
    if (selected.length && !selected.includes(name)) continue;
    const file = join(scratch, path),
      source = readFileSync(file, 'utf8');
    let changed = source;
    if (mode === 'launder') {
      changed = source
        .replace("if (rowValues.includes('FAIL')) return [definition.id, 'FAIL'];", '')
        .replace(
          'if (!stopped && windowsFailed.some((name) => !retryRow(name))) {',
          'if (false) {',
        );
    } else if (mode === 'proposal')
      changed = source
        .replace("if (\n    value.status !== 'executable' ||", 'if (\n    false ||')
        .replace("row.status !== 'executable' ||", 'false ||')
        .replace('row.blockers.length !== 0 ||', 'false ||')
        .replace("        row.windows_guest_abi === 'UNKNOWN' ||", 'false ||')
        .replace("Object.values(row.minimum_drivers).some((v) => v === 'UNKNOWN') ||", 'false ||');
    else if (mode === 'reporter-cap' || mode === 'payload-cap') {
      const pattern =
        mode === 'reporter-cap'
          ? /if \(!\/\^BOX_RELEASE_MATRIX_RECEIPTS[\s\S]*?refuse\('NOT_ELIGIBLE', 409, 'MATRIX_REPORTER_CAPABILITY'\);/
          : /if \(!\/\^PAYLOAD_RELEASE_MATRIX_EXECUTOR[\s\S]*?refuse\('NOT_ELIGIBLE', 409, 'MATRIX_PAYLOAD_CAPABILITY'\);/;
      if (!pattern.test(source)) throw Error('cap anchor ' + name);
      changed = source.replace(pattern, '');
    } else {
      if (source.split(before).length !== 2) throw Error('mutant anchor ' + name);
      changed = source.replace(before, after);
    }
    writeFileSync(file, changed);
    let killers;
    try {
      killers = run(mode === 'python' ? 'python' : 'node');
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
