import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  RELEASE_MATRIX,
  validateReleaseMatrix,
  releaseEnvironment,
} from '../src/release-matrix.js';
import { DEFAULTS } from '../src/defaults.js';
import { executableMatrix } from './release-fixture.js';
import { classifyLeg } from '../src/release.js';
const check = (m) => validateReleaseMatrix(m, DEFAULTS);
const candidate = () => structuredClone(RELEASE_MATRIX);
const pin = (m) => m.driver_installers.find((p) => p.status === 'PINNED' && p.os === 'linux');
// FALSIFIER: malformed/missing media validates, an unpinned executable validates,
// or media drift can acquire an ordinary PASS despite all passing summary rows.
test('pins: proposal records every full, subrun and cross installer', () => {
  assert.deepEqual(check(RELEASE_MATRIX), []);
  assert.equal(RELEASE_MATRIX.driver_installers.length, 11);
  for (const row of RELEASE_MATRIX.dies) {
    const decoded = JSON.parse(
      Buffer.from(releaseEnvironment(row).KF_RELEASE_DRIVER_PINS, 'base64url'),
    );
    assert.ok(decoded.length >= 3);
    for (const drivers of [row.drivers, ...(row.cross_subrun ? [row.cross_subrun.drivers] : [])])
      for (const [axis, version] of Object.entries(drivers))
        assert.ok(
          decoded.some(
            (p) =>
              p.os === (axis === 'windows_guest_driver' ? 'windows' : 'linux') &&
              p.version === version,
          ),
        );
    for (const subrun of row.subruns)
      for (const version of subrun.drivers)
        assert.ok(decoded.some((p) => p.os === subrun.os && p.version === version));
  }
});
for (const [name, mutate] of [
  ['missing catalog', (m) => delete m.driver_installers],
  ['null pin', (m) => (m.driver_installers[0] = null)],
  ['scalar catalog', (m) => (m.driver_installers = false)],
  [
    'missing full pin',
    (m) =>
      m.driver_installers.splice(
        m.driver_installers.findIndex((p) => p.version === '580.178.04'),
        1,
      ),
  ],
  [
    'missing subrun pin',
    (m) =>
      m.dies[0].subruns.push({
        os: 'linux',
        driver_family: '580',
        profile: 'small',
        drivers: ['580.119.02'],
      }),
  ],
  ['http', (m) => (pin(m).url = pin(m).url.replace('https:', 'http:'))],
  [
    'unofficial host',
    (m) => (pin(m).url = pin(m).url.replace('download.nvidia.com', 'drivers.example.com')),
  ],
  [
    'deceptive host',
    (m) =>
      (pin(m).url = pin(m).url.replace('download.nvidia.com', 'download.nvidia.com.example.com')),
  ],
  [
    'credentials',
    (m) => {
      const u = new URL(pin(m).url);
      u.username = 'fixture-user';
      u.password = 'fixture-password';
      pin(m).url = u.href;
    },
  ],
  ['query', (m) => (pin(m).url += '?temporary=locator')],
  ['wrong version URL', (m) => (pin(m).url = pin(m).url.replaceAll(pin(m).version, '580.65.06'))],
  ['invalid hash', (m) => (pin(m).sha256 = 'not-a-sha256')],
  ['zero size', (m) => (pin(m).size = 0)],
  ['fractional size', (m) => (pin(m).size = 0.5)],
  ['duplicate pin', (m) => m.driver_installers.push(structuredClone(pin(m)))],
  [
    'unofficial catalog binary',
    (m) => {
      pin(m).os = 'windows';
      pin(m).url = 'https://downloads.microsoft.com/driver.cab';
    },
  ],
  [
    'catalog without evidence',
    (m) => {
      const p = m.driver_installers.find((p) => p.os === 'windows');
      p.url = 'https://catalog.s.download.windowsupdate.com/driver.cab';
      p.catalog_url = null;
    },
  ],
  [
    'unpinned executable',
    (m) => {
      m.status = 'executable';
      const p = pin(m);
      p.status = 'UNPINNED';
      p.sha256 = null;
      p.size = null;
    },
  ],
]) {
  test('pins rejects ' + name, () => {
    const m = candidate();
    mutate(m);
    assert.notDeepEqual(check(m), []);
  });
  test('pins CLI rejects ' + name, () => {
    const m = candidate();
    mutate(m);
    const dir = mkdtempSync(join(tmpdir(), 'kf-J2-pin-case-'));
    try {
      const file = join(dir, 'matrix.json');
      writeFileSync(file, JSON.stringify(m));
      const run = spawnSync(
        process.execPath,
        [new URL('../validate-release-matrix.mjs', import.meta.url).pathname, file],
        { encoding: 'utf8', timeout: 60000 },
      );
      assert.equal(run.status, 1, run.stdout + run.stderr);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
}
test('pins: all official NVIDIA download hosts are allowed', () => {
  for (const host of [
    'download.nvidia.com',
    'us.download.nvidia.com',
    'international.download.nvidia.com',
  ]) {
    const m = candidate();
    pin(m).url = pin(m).url.replace('download.nvidia.com', host);
    assert.deepEqual(check(m), []);
  }
});
test('pins: executable fixture must resolve all installers', () => {
  const m = executableMatrix();
  assert.ok(m.driver_installers.every((p) => p.status === 'PINNED'));
  assert.deepEqual(check(m), []);
});
for (const name of ['driver_media_drift', 'windows_driver_media_drift'])
  test('media drift prevents PASS for ' + name, () => {
    const record = {
      ended: 1,
      verdict: 'PASS',
      hardware: { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' },
      result: {
        verdict: 'PASS',
        host_reference: 'GREEN',
        suites: [
          { name: 'cuda_fat', pass: 1, fail: 0 },
          { name, pass: 0, fail: 1 },
        ],
      },
    };
    const options = { windowsRuns: 0, die: 'AD104', family: 'ada', cfg: DEFAULTS };
    const clean = structuredClone(record);
    clean.result.suites.pop();
    assert.equal(classifyLeg(clean, options).verdict, 'PASS');
    const result = classifyLeg(record, options);
    assert.equal(result.verdict, 'RETRY');
    assert.equal(result.reason, 'MEDIA_DRIFT');
  });

for (const die of ['TU116', 'GB205'])
  test('pins: plan exposes the payload pins for ' + die, async () => {
    const { offlinePlan } = await import('../release-plan.mjs');
    const { readFileSync } = await import('node:fs');
    const market = JSON.parse(
      readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
    );
    const { plan } = {
      plan: await offlinePlan(market.offers, {
        RELEASE_ENABLED: 'true',
        MAX_CONCURRENT_RENTALS: '9',
      }),
    };
    for (const leg of plan.families
      .flatMap((f) => f.cells.flatMap((c) => c.legs))
      .filter((l) => l.release_matrix.die === die)) {
      const decoded = JSON.parse(
        Buffer.from(releaseEnvironment(leg.release_matrix).KF_RELEASE_DRIVER_PINS, 'base64url'),
      );
      assert.deepEqual(leg.driver_pins, decoded);
      assert.ok(decoded.length > 0);
    }
  });

test('pins: Windows Catalog requires its own official catalog record', () => {
  const m = candidate();
  const p = m.driver_installers.find((p) => p.os === 'windows');
  p.url = 'https://catalog.s.download.windowsupdate.com/fixture.cab';
  p.catalog_url =
    'https://www.catalog.update.microsoft.com/ScopedViewInline.aspx?updateid=11111111-1111-1111-1111-111111111111';
  p.source = 'SYNTHETIC schema fixture, never an installer pin used for hardware';
  assert.deepEqual(check(m), []);
  p.os = 'linux';
  assert.notDeepEqual(check(m), []);
});
