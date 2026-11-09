#!/usr/bin/env node
// Falsifier: any official-pin/media-drift mutation has fewer than two named killers.
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
const broker = fileURLToPath(new URL('.', import.meta.url));
const repo = fileURLToPath(new URL('../../', import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), 'kf-J2-driver-pin-mutants-'));
const V = 'ci/vast-broker/src/release-matrix.js',
  L = 'ci/vast-broker/src/release.js',
  P = 'scripts/ci/release_driver_media.py';
const cases = [
  [
    'drift receipt duplicated',
    'scripts/ci/vast_payload.py',
    'raise EvidenceError("driver_media_drift")',
    'raise EvidenceError("driver_media_drift", [row("driver_media_drift", failed=1)])',
    'python',
  ],

  ['media drift masks attributed failure', L, 'if (realFailure)', 'if (false)', 'node'],

  [
    'filesystem drift escapes reporting',
    P,
    '    except (MediaDrift, OSError, ValueError) as error:',
    '    except MediaDrift as error:',
    'python',
  ],

  ['insecure installer scheme accepted', V, "u.protocol !== 'https:' ||", 'false ||', 'node'],
  [
    'unofficial installer host accepted',
    V,
    'NVIDIA_INSTALLER_HOSTS.has(u.hostname)',
    'true',
    'node',
  ],
  ['installer hash shape ignored', V, '/^[a-f0-9]{64}$/.test(pin.sha256)', 'true', 'node'],
  ['zero installer size accepted', V, 'pin.size > 0 &&', 'true &&', 'node'],
  [
    'selected installer coverage ignored',
    V,
    '[...driverInstallerKeys(row)].every((id) => installerKeys.has(id))',
    'true',
    'node',
  ],
  ['duplicate installer accepted', V, 'bad(!installerKeys.has(id),', 'bad(true,', 'node'],
  [
    'UNPINNED matrix executable',
    V,
    "value.driver_installers.every((p) => p?.status === 'PINNED')",
    'true',
    'node',
  ],
  [
    'installer environment omitted',
    V,
    'KF_RELEASE_DRIVER_PINS: btoa(',
    'KF_RELEASE_DRIVER_PINS_DROPPED: btoa(',
    'node',
  ],
  [
    'plan pins omitted',
    L,
    'driver_pins: structuredClone(driverPinsFor(matrixRow, releaseMatrix))',
    'driver_pins: []',
    'node',
  ],
  [
    'media drift classified as PASS',
    L,
    'if (rows.some((r) => r.fail > 0 && /^(?:[a-z0-9]+_)*driver_media_drift$/.test(r.name)))',
    'if (false)',
    'node',
  ],
  [
    'download hash not checked',
    P,
    "if digest.hexdigest() != pin['sha256']:",
    'if False:',
    'python',
  ],
  [
    'cached installer not checked',
    P,
    '        checked_file(pin, target)',
    '        pass',
    'python',
  ],
  ['UNPINNED media fetched', P, "if pin['status'] != 'PINNED':", 'if False:', 'python'],
  [
    '404 media drift swallowed',
    P,
    "        raise MediaDrift('HTTP_' + str(error.code)) from None",
    '        return target',
    'python',
  ],
  ['redirect accepted', P, "if response.geturl() != pin['url']:", 'if False:', 'python'],
  ['media drift CLI succeeds', P, '        return 3', '        return 0', 'python'],
  [
    'host media guard bypassed',
    'scripts/bench/provision_host_driver.sh',
    'if [ -n "${KF_RELEASE_DRIVER_PINS:-}" ]; then',
    'if false; then',
    'python',
  ],
  [
    'provision media drift ignored',
    'scripts/bench/box/provision_full.sh',
    '  exit 3\nfi\n# KF_DRIVER_MEDIA_GATE_END',
    '  :\nfi\n# KF_DRIVER_MEDIA_GATE_END',
    'python',
  ],
  [
    'payload loses media drift reason',
    'scripts/ci/vast_payload.py',
    '    if re.search(r"^DRIVER_MEDIA_DRIFT reason=[A-Z0-9_]{1,32}$", text, re.M):',
    '    if False:',
    'python',
  ],
  [
    'payload installer coverage ignored',
    'scripts/ci/vast_payload.py',
    '    if not required.issubset({(p["os"], p["version"]) for p in pins}):',
    '    if False:',
    'python',
  ],
];
function run(mode) {
  const python = mode === 'python';
  const args = python
    ? [
        '-c',
        `import io,json,unittest
s=unittest.defaultTestLoader.discover('scripts/ci',pattern='test_release_driver_media.py')
r=unittest.TextTestRunner(stream=io.StringIO()).run(s)
print(json.dumps({'killers':sorted(set(t.id() for t,_ in r.failures+r.errors))}))`,
      ]
    : ['--test', '--test-reporter=tap', 'ci/vast-broker/test/driver-pins.test.js'];
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
  for (const path of [
    'scripts/ci/vast_report.py',
    'scripts/ci/release_driver_pins.py',
    'scripts/ci/vast_payload.py',
    'scripts/ci/release_driver_media.py',
    'scripts/ci/test_release_driver_media.py',
    'scripts/bench/native_xid_guard.py',
    'scripts/bench/native_fault_names.json',
    'scripts/bench/provision_host_driver.sh',
    'scripts/bench/box/provision_full.sh',
  ]) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path));
  }
  for (const mode of ['node', 'python']) {
    const failures = run(mode);
    if (failures.length) throw Error('baseline red ' + JSON.stringify(failures));
  }
  const rows = [];
  for (const [name, path, before, after, mode] of cases) {
    const file = join(scratch, path),
      source = readFileSync(file, 'utf8');
    if (source.split(before).length !== 2) throw Error('mutant anchor ' + name);
    writeFileSync(file, source.replace(before, after));
    let killers;
    try {
      killers = run(mode);
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
