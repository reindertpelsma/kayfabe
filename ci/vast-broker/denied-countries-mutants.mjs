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
const broker = fileURLToPath(new URL('.', import.meta.url)),
  repo = fileURLToPath(new URL('../..', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-countries-'));
const O = 'src/offers.js',
  P = 'src/policy.js';
const edit = (name, file, before, after) => ({ name, file, before, after });
const cases = [
  ...['RU', 'BY'].map((code) => ({
    name: 'default omits ' + code,
    change(d) {
      d.deniedCountries = d.deniedCountries.filter((x) => x !== code);
    },
  })),
  {
    name: 'default order changed',
    change(d) {
      const i = d.deniedCountries.indexOf('RU');
      d.deniedCountries.splice(i, 2, 'BY', 'RU');
    },
  },
  edit('RU names omitted', O, "RU: ['RU', 'RUS', 'Russia', 'Russian Federation']", "RU: ['RU']"),
  edit('BY names omitted', O, "BY: ['BY', 'BLR', 'Belarus', 'Republic of Belarus']", "BY: ['BY']"),
  ...['RU', 'BY'].map((code) =>
    edit(
      code + ' filter omitted',
      O,
      'cfg.deniedCountries.includes(country)',
      "(cfg.deniedCountries.includes(country) && country !== '" + code + "')",
    ),
  ),
  ...['RU', 'BY'].map((code) =>
    edit(
      code + ' override ignored',
      O,
      'cfg.deniedCountries.includes(country)',
      "(cfg.deniedCountries.includes(country) || country === '" + code + "')",
    ),
  ),
  edit(
    'duplicate codes accepted',
    P,
    'new Set(deniedCountries).size !== deniedCountries.length',
    'false',
  ),
  edit(
    'three-letter codes accepted',
    P,
    'deniedCountries.every((x) => /^[A-Z]{2}$/.test(x))',
    'deniedCountries.every((x) => /^[A-Z]{2,3}$/.test(x))',
  ),
  edit(
    'lowercase codes accepted',
    P,
    'deniedCountries.every((x) => /^[A-Z]{2}$/.test(x))',
    'deniedCountries.every((x) => /^[A-Z]{2}$/i.test(x))',
  ),
  edit('COUNTRY reason changed', O, "reasons.push('COUNTRY')", "reasons.push('OTHER_COUNTRY')"),
];
function run() {
  const r = spawnSync(
    process.execPath,
    ['--test', '--test-reporter=tap', 'test/edits-countries.test.js'],
    {
      cwd: join(scratch, 'ci/vast-broker'),
      encoding: 'utf8',
      timeout: 120000,
      maxBuffer: 8 * 1024 * 1024,
    },
  );
  if (r.error) throw r.error;
  const failures = [];
  for (const block of r.stdout.split(/^# Subtest: /m)) {
    const name = block.match(/^not ok \d+ - (.+)$/m)?.[1];
    if (!name) continue;
    if (!/code: 'ERR_ASSERTION'/.test(block)) throw Error('non-assertion failure: ' + name);
    failures.push(name);
  }
  if (r.status !== 0 && !failures.length) throw Error(r.stdout + r.stderr);
  return {
    status: r.status,
    tests: Number(r.stdout.match(/^# tests (\d+)$/m)?.[1]),
    failures: [...new Set(failures)],
  };
}
try {
  cpSync(broker, join(scratch, 'ci/vast-broker'), {
    recursive: true,
    filter: (p) => !p.includes('/node_modules') && !p.includes('/.wrangler'),
  });
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  for (const path of [
    'scripts/ci/vast_report.py',
    'scripts/ci/vast_payload.py',
    'scripts/ci/release_driver_pins.py',
    'ci/vast-reaper',
    'ci/platform-api/migrations',
    'ci/requirements.toml',
  ]) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path), { recursive: true });
  }
  const baseline = run();
  if (baseline.status !== 0) throw Error('baseline failed: ' + JSON.stringify(baseline));
  const rows = [];
  for (const c of cases) {
    const path = join(scratch, 'ci/vast-broker', c.change ? 'defaults.json' : c.file),
      original = readFileSync(path, 'utf8');
    try {
      if (c.change) {
        const d = JSON.parse(original);
        c.change(d);
        writeFileSync(path, JSON.stringify(d));
      } else {
        if (original.split(c.before).length !== 2) throw Error('ambiguous edit ' + c.name);
        writeFileSync(path, original.replace(c.before, c.after));
      }
      const r = run();
      if (r.status !== 1 || r.failures.length < 2)
        throw Error('fewer than two killers: ' + c.name + ' ' + JSON.stringify(r));
      rows.push({ mutant: c.name, kills: r.failures.length, killers: r.failures });
    } finally {
      writeFileSync(path, original);
    }
  }
  process.stdout.write(
    JSON.stringify({ status: 'PASS', baseline_tests: baseline.tests, cases: rows }, null, 2) + '\n',
  );
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
