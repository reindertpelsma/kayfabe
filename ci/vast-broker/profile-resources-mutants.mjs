#!/usr/bin/env node
// GPU-free, isolated mutations. At least two named test killers per new resource rule.
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
const repo = fileURLToPath(new URL('../../', import.meta.url));
const broker = fileURLToPath(new URL('.', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-resource-mutants-'));
const P = 'ci/vast-broker/src/policy.js',
  I = 'ci/vast-broker/src/index.js',
  V = 'ci/vast-broker/src/vast.js',
  L = 'ci/vast-broker/src/release.js',
  G = 'scripts/ci/profile_resources.py';
const cases = [
  [
    'large sanity cap remains narrow',
    'ci/vast-broker/defaults.json',
    '"maxInetUsdPerGb": 0.02',
    '"maxInetUsdPerGb": 0.01',
  ],
  [
    'profile allowances ignored',
    P,
    'const base = DEFAULTS.payloadProfiles[name];',
    'const base = DEFAULTS.payloadProfiles.small;',
  ],
  ['inbound allowance omitted', P, 'resources.bandwidthDownGb * offer.inet_down_cost', '0'],
  [
    'inbound per-GB cap removed',
    P,
    'offer.inet_down_cost > resources.maxInetUsdPerGb ||',
    'false ||',
  ],
  [
    'outbound per-GB cap removed',
    P,
    'offer.inet_up_cost > resources.maxInetUsdPerGb ||',
    'false ||',
  ],
  [
    'live per-GB checks removed',
    I,
    ['instance.inet_up_cost > cfg.maxInet ||', 'instance.inet_down_cost > cfg.maxInet ||'],
    ['false ||', 'false ||'],
  ],
  [
    'large disk ignored at provider',
    V,
    'this.cfg.diskGb ?? DEFAULTS.payloadProfiles.small.diskGb',
    'DEFAULTS.payloadProfiles.small.diskGb',
    'all',
  ],
  [
    'storage quote remains on old disk',
    P,
    'offer.storage_cost * resources.diskGb',
    'offer.storage_cost * DEFAULTS.payloadProfiles.small.diskGb',
  ],
  [
    'release ranking ignores network',
    L,
    'a.pricing.effective - b.pricing.effective',
    'a.dph_base - b.dph_base',
  ],
  [
    'unbudgeted stages accepted',
    G,
    'missing = stage_names(source) - stages.keys()',
    'missing = set()',
  ],
  [
    'stage totals ignored',
    G,
    'if profile.get(key) != math.ceil(sum(s[key] for s in active)):',
    'if False:',
  ],
];
const collect = `import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_vast_profile_resources.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'tests':r.testsRun,'killers':[t.id() for t,_ in r.failures+r.errors]}))`;
function run(python) {
  const result = spawnSync(
    python ? 'python3' : process.execPath,
    python
      ? ['-c', collect]
      : ['--test', '--test-reporter=tap', 'ci/vast-broker/test/profile-resources.test.js'],
    { cwd: scratch, encoding: 'utf8', timeout: 120000 },
  );
  if (result.error) throw result.error;
  if (python) {
    if (result.status !== 0) throw Error(result.stderr);
    return JSON.parse(result.stdout.trim()).killers;
  }
  const killers = [...result.stdout.matchAll(/^not ok \d+ - (.+)$/gm)]
    .map((m) => m[1])
    .filter((n) => !n.endsWith('.test.js'));
  if (result.status !== 0 && !killers.length) throw Error(result.stderr + result.stdout);
  return [...new Set(killers)];
}
try {
  cpSync(broker, join(scratch, 'ci/vast-broker'), {
    recursive: true,
    filter: (p) => !p.includes('node_modules') && !p.includes('/.wrangler') && !p.includes('/dist'),
  });
  symlinkSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), 'dir');
  const defaults = JSON.parse(readFileSync(join(repo, 'ci/vast-broker/defaults.json')));
  const files = new Set([
    'scripts/ci/vast_report.py',
    'scripts/ci/release_driver_pins.py',
    'scripts/ci/vast_payload.py',
    'scripts/bench/native_xid_guard.py',
    G,
    'scripts/ci/test_vast_profile_resources.py',
    ...Object.values(defaults.payloadStageResources).map((s) => s.source),
  ]);
  for (const path of files) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path));
  }
  for (const python of [false, true]) {
    const killers = run(python);
    if (killers.length) throw Error('baseline red: ' + JSON.stringify(killers));
  }
  const rows = [];
  for (const [name, path, before, after, mode] of cases) {
    const file = join(scratch, path),
      source = readFileSync(file, 'utf8');
    let changed = source;
    const replacements = Array.isArray(before)
      ? before.map((b, i) => [b, after[i]])
      : [[before, after]];
    for (const [b, a] of replacements) {
      if (!changed.includes(b) || (mode !== 'all' && changed.split(b).length !== 2))
        throw Error('mutant anchor: ' + name);
      changed = mode === 'all' ? changed.split(b).join(a) : changed.replace(b, a);
    }
    writeFileSync(file, changed);
    let killers;
    try {
      killers = run(path.endsWith('.py'));
    } finally {
      writeFileSync(file, source);
    }
    if (killers.length < 2)
      throw Error(name + ': fewer than two killers ' + JSON.stringify(killers));
    rows.push({ mutant: name, kills: killers.length, killers });
  }
  console.log(JSON.stringify({ status: 'PASS', cases: rows }, null, 2));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
