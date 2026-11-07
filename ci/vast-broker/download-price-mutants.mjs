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
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-download-mutants-'));
const P = 'src/policy.js',
  I = 'src/index.js',
  M = 'src/download-metrics.js';
const cases = [
  [
    'provision_full metric forwarding suppressed',
    '../../scripts/bench/box/provision_full.sh',
    'emit_download_metrics() { LC_ALL=C grep -a -m 32 -E \'^KF_DOWNLOAD_METRIC seconds=[0-9]{1,6}([.][0-9]{1,6})? bytes=[0-9]{1,13}$\' -- "$1" 2>/dev/null || true; }',
    'emit_download_metrics() { :; }',
  ],
  [
    'windows_gate metric forwarding suppressed',
    '../../scripts/bench/box/windows_gate.sh',
    'emit_download_metrics() { LC_ALL=C grep -a -m 32 -E \'^KF_DOWNLOAD_METRIC seconds=[0-9]{1,6}([.][0-9]{1,6})? bytes=[0-9]{1,13}$\' -- "$1" 2>/dev/null || true; }',
    'emit_download_metrics() { :; }',
  ],

  [
    'short lease feasibility ignored',
    P,
    'max_seconds: Math.min(resource.maxDownloadSeconds, cfg.hard / 1000),',
    'max_seconds: resource.maxDownloadSeconds,',
  ],
  [
    'time premium omitted',
    P,
    'const downloadTimeCost = (download.extra_seconds / 3600) * computeStorage;',
    'const downloadTimeCost = 0;',
  ],
  [
    'storage excluded from time premium',
    P,
    'const downloadTimeCost = (download.extra_seconds / 3600) * computeStorage;',
    'const downloadTimeCost = (download.extra_seconds / 3600) * offer.dph_base;',
  ],
  [
    'fast link receives negative charge',
    P,
    'extra_seconds: Math.max(0, expected - reference),',
    'extra_seconds: expected - reference,',
  ],
  ['derate hardcoded', P, '(model.derate * offer.inet_down)', '(0.3 * offer.inet_down)'],
  ['reference hardcoded', P, '/ model.referenceDownMbps', '/ 1000'],
  ['feasibility guard removed', P, "reasons.push('SLOW_NETWORK');", 'void 0;'],
  [
    'profile download limit ignored',
    P,
    'price.download.expected_seconds > price.download.max_seconds',
    'price.download.expected_seconds > 3600',
  ],
  [
    'invalid speed priced',
    P,
    'if (!Number.isFinite(offer.inet_down) || offer.inet_down <= 0) return null;',
    'void 0;',
  ],
  ['nonfinite duration priced', P, 'if (!Number.isFinite(expected)) return null;', 'void 0;'],
  ['download model schema unchecked', P, "refuse('CONFIG_VAR:DOWNLOAD_MODEL', 503);", 'void 0;'],
  ['download limit schema unchecked', P, "refuse('CONFIG_VAR:DOWNLOAD_LIMIT', 503);", 'void 0;'],
  [
    'live feasibility close omitted',
    I,
    "await close(record, vast, 'CONTRACT_SLOW_NETWORK', 'UNAVAILABLE');",
    'void 0;',
  ],
  [
    'live advertised fallback omitted',
    I,
    'instance.inet_down ?? record.offerFacts?.inet_down',
    'instance.inet_down',
  ],
  [
    'release estimate drops time premium',
    'src/release.js',
    'est_usd: round(o.pricing.total),',
    'est_usd: round(o.pricing.total - o.pricing.downloadTimeCost),',
  ],
  ['download metric ingestion omitted', I, 'captureDownloadMetrics(r, payload.text);', 'void 0;'],
  ['arbitrary partial logs retained', M, 'if (!partialMetric(tail)) {', 'if (false) {'],
  [
    'measurement count unbounded',
    M,
    'measurements.samples.length >= 32',
    'measurements.samples.length >= 320',
  ],
  [
    'oversized measurement accepted',
    M,
    'if (seconds > 86400 || bytes > 1e12) continue;',
    'void 0;',
  ],
  [
    'measurement settlement snapshot lost',
    'src/ledger.js',
    'downloadMeasurements: structuredClone(record.downloadMeasurements ?? null),',
    'downloadMeasurements: null,',
  ],
  [
    'measurement publisher snapshot lost',
    'src/publish.js',
    'downloadMeasurements: entry.downloadMeasurements ?? null,',
    'downloadMeasurements: null,',
    'publish',
  ],
  [
    'measurement report field lost',
    'src/runreport.js',
    'download_measurements: downloadMeasurements,',
    'download_measurements: null,',
    'publish',
  ],
  [
    'producer timing suppressed',
    '../../scripts/bench/box/fetch_bench_data.sh',
    'printf \'%s\\n\' "$metric"',
    ': "$metric"',
    'shell',
  ],
  [
    'slow network code unsupported by runner',
    'runner.py',
    '        "SLOW_NETWORK",\n',
    '',
    'codes',
  ],
];
function run(mode) {
  const args =
    mode === 'shell'
      ? ['scripts/bench/box/test_fetch_bench_data.sh']
      : mode === 'publish'
        ? [
            '--test',
            '--test-reporter=tap',
            '--test-name-pattern',
            'download measurements publish',
            'ci/vast-broker/test/run-report.test.js',
          ]
        : mode === 'codes'
          ? ['--test', '--test-reporter=tap', 'ci/vast-broker/test/edits2-identity.test.js']
          : [
              '--test',
              '--test-reporter=tap',
              'ci/vast-broker/test/download-price.test.js',
              'ci/vast-broker/test/download-metrics.test.js',
              'ci/vast-broker/test/download-forwarding.test.js',
            ];
  const r = spawnSync(mode === 'shell' ? 'bash' : process.execPath, args, {
    cwd: scratch,
    encoding: 'utf8',
    timeout: 300000,
  });
  if (r.error) throw r.error;
  const output = r.stdout + '\n' + r.stderr;
  const killers = [...output.matchAll(mode === 'shell' ? /^FAIL (.+)$/gm : /^not ok \d+ - (.+)$/gm)]
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
  for (const relative of [
    'scripts/ci/vast_report.py',
    'ci/requirements.toml',
    'scripts/bench/box/fetch_bench_data.sh',
    'scripts/bench/box/test_fetch_bench_data.sh',
    'scripts/bench/box/provision_full.sh',
    'scripts/bench/box/windows_gate.sh',
  ]) {
    mkdirSync(dirname(join(scratch, relative)), { recursive: true });
    cpSync(join(root, relative), join(scratch, relative));
  }
  for (const mode of ['unit', 'publish', 'shell', 'codes']) {
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
