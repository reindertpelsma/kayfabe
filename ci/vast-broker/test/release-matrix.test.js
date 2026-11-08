import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import matrix from '../release-matrix.json' with { type: 'json' };
import { DEFAULTS } from '../src/defaults.js';
import {
  validateReleaseMatrix,
  hostProvidesDrivers,
  releaseEnvironment,
} from '../src/release-matrix.js';

// Falsifiers: duplicate combinations or wrong die coverage validate; a fixed-driver host
// matches a swap requirement; a proposed subrun disappears in the box environment.
const copy = () => structuredClone(matrix);
const check = (m) => validateReleaseMatrix(m, DEFAULTS);
test('matrix: proposed eight-die fixture validates and all minima stay explicit', () => {
  assert.deepEqual(check(matrix), []);
  assert.equal(matrix.dies.length, 8);
  assert.ok(
    matrix.dies.every(
      (d) =>
        d.status === 'proposed' && Object.values(d.minimum_drivers).every((v) => v === 'UNKNOWN'),
    ),
  );
});
test('matrix: every pair and all driver combinations are explicit', () => {
  assert.deepEqual(
    matrix.dies.map((d) => d.die),
    ['TU106', 'TU116', 'GA102', 'GA104', 'AD102', 'AD104', 'GB202', 'GB205'],
  );
  assert.equal(new Set(matrix.dies.map((d) => JSON.stringify(d.drivers))).size, 8);
  for (const axis of ['linux_guest_driver', 'windows_guest_driver'])
    assert.equal(
      new Set(matrix.dies.map((d) => d.drivers.host_driver + '/' + d.drivers[axis])).size,
      8,
    );
  assert.equal(matrix.dies.filter((d) => d.cross_subrun !== null).length, 4);
});
for (const [name, mutate, error] of [
  ['die count', (m) => m.dies.pop(), 'die count'],
  ['die balance', (m) => (m.dies[0].die_family = 'ampere'), 'die family'],
  [
    'duplicate drivers',
    (m) => {
      m.dies[2].drivers = structuredClone(m.dies[0].drivers);
      m.dies[3].cross_subrun.drivers = structuredClone(m.dies[0].drivers);
    },
    'unique driver combinations',
  ],
  [
    'unknown die',
    (m) => {
      m.dies[0].die = 'TU102';
      m.dies[1].cross_subrun.die = 'TU102';
    },
    'releaseDies',
  ],
  ['minimum host', (m) => (m.dies[0].minimum_drivers.host_driver = '590.48.01'), 'minimum driver'],
  [
    'minimum guest',
    (m) => (m.dies[0].minimum_drivers.linux_guest_driver = '610.43.02'),
    'minimum driver',
  ],
  ['profile', (m) => (m.dies[0].subruns[0].profile = 'full'), 'subrun profile'],
  ['driver major', (m) => (m.dies[0].subruns[0].driver_family = '570'), 'driver family'],
  ['missing cross', (m) => (m.dies[1].cross_subrun = null), 'cross subrun'],
  [
    'wrong cross',
    (m) => (m.dies[1].cross_subrun.drivers.host_driver = '580.105.08'),
    'cross subrun',
  ],
  ['unknown field', (m) => (m.dies[0].drivers.guest_driver = '580.88'), 'schema'],
  ['driver injection', (m) => (m.dies[0].drivers.host_driver += '\n'), 'driver version'],
]) {
  test(`matrix: rejects ${name}`, () => {
    const m = copy();
    mutate(m);
    assert.ok(
      check(m).some((e) => e.includes(error)),
      check(m).join(';'),
    );
  });
  test(`matrix: ${name} cannot pass the CLI contract`, () => {
    const m = copy();
    mutate(m);
    const temporary = mkdtempSync(join(tmpdir(), 'kf-J2-invalid-matrix-'));
    try {
      const candidate = join(temporary, 'candidate.json');
      writeFileSync(candidate, JSON.stringify(m));
      const run = spawnSync(
        process.execPath,
        [new URL('../validate-release-matrix.mjs', import.meta.url).pathname, candidate],
        { encoding: 'utf8', timeout: 60000 },
      );
      assert.equal(run.status, 1, run.stderr);
      const result = JSON.parse(run.stdout);
      assert.equal(result.status, 'INVALID');
      assert.ok(result.errors > 0);
    } finally {
      rmSync(temporary, { recursive: true, force: true });
    }
  });
}
test('host capability: a root host can supply all required host drivers', () => {
  assert.equal(hostProvidesDrivers({ can_swap_host_driver: true }, matrix.dies[1]), true);
  assert.equal(
    hostProvidesDrivers(
      { can_swap_host_driver: false, host_driver: matrix.dies[1].drivers.host_driver },
      matrix.dies[1],
    ),
    false,
    'cross subrun also changes host driver',
  );
});
test('host capability: fixed hosts must match the whole run and absence is not permission', () => {
  const row = matrix.dies[0];
  assert.equal(
    hostProvidesDrivers({ can_swap_host_driver: false, host_driver: row.drivers.host_driver }, row),
    true,
  );
  assert.equal(
    hostProvidesDrivers({ can_swap_host_driver: false, host_driver: '535.309.01' }, row),
    false,
  );
  assert.equal(hostProvidesDrivers({}, row), false);
});
test('environment: independent driver fields and complete subrun plan survive JSON transport', () => {
  const d = matrix.dies[1],
    e = releaseEnvironment(d);
  assert.equal(e.KF_HOST_DRIVER, d.drivers.host_driver);
  assert.equal(e.KF_LINUX_GUEST_DRIVER, d.drivers.linux_guest_driver);
  assert.equal(e.KF_WINDOWS_GUEST_DRIVER, d.drivers.windows_guest_driver);
  assert.deepEqual(JSON.parse(Buffer.from(e.KF_RELEASE_SUBRUNS, 'base64url')), {
    subruns: d.subruns,
    cross_subrun: d.cross_subrun,
    status: 'proposed',
  });
});
test('environment: mapping never conflates Windows media with its ABI', () => {
  for (const d of matrix.dies) {
    const e = releaseEnvironment(d);
    assert.equal(
      e.KF_WINGATE_GUEST_DRIVER,
      d.windows_guest_abi === 'UNKNOWN' ? undefined : d.windows_guest_abi,
    );
    assert.notEqual(e.KF_WINDOWS_GUEST_DRIVER, e.KF_WINGATE_GUEST_DRIVER);
    assert.deepEqual(JSON.parse(Buffer.from(e.KF_RELEASE_SUBRUNS, 'base64url')).subruns, d.subruns);
  }
});

import { offlinePlan } from '../release-plan.mjs';
import { readFileSync } from 'node:fs';
import { tierEnvironment } from '../src/tier.js';
import { dieLegs } from '../src/release.js';
const marketFixture = JSON.parse(
  readFileSync(new URL('./fixtures/release-market-synthetic.json', import.meta.url)),
);
const releaseVars = { RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '9' };
test('planner fixture: exactly eight full legs retain every driver and subrun', async () => {
  const plan = await offlinePlan(marketFixture.offers, releaseVars);
  assert.equal(plan.legs, 8);
  assert.equal(plan.parallel, 8);
  assert.equal(plan.execution_ready, false);
  assert.equal(plan.matrix_status, 'proposed');
  const legs = plan.families.flatMap((f) => f.cells.flatMap((c) => c.legs));
  assert.deepEqual(
    legs.map((l) => l.release_matrix),
    matrix.dies,
  );
  for (const [i, l] of legs.entries()) {
    assert.deepEqual(l.drivers, matrix.dies[i].drivers);
    assert.deepEqual(l.subruns, matrix.dies[i].subruns);
    assert.deepEqual(l.cross_subrun, matrix.dies[i].cross_subrun);
    assert.equal(l.profile, 'full');
    assert.equal(l.windows_runs, 2);
  }
});
test('planner fixture: per-leg payload selection is the matrix selection', async () => {
  const plan = await offlinePlan(marketFixture.offers, releaseVars);
  for (const [i, l] of plan.families.flatMap((f) => f.cells.flatMap((c) => c.legs)).entries()) {
    assert.deepEqual(l.release_matrix, matrix.dies[i]);
    assert.deepEqual(tierEnvironment('release', { releaseMatrix: l.release_matrix }), {
      KF_MERGE_WINDOWS: '1',
      ...releaseEnvironment(matrix.dies[i]),
    });
  }
  assert.throws(
    () => tierEnvironment('check', { releaseMatrix: matrix.dies[0] }),
    /release matrix profile/,
  );
});
test('host capability: a fixed second die cannot skip the cross-driver requirement', () => {
  for (const row of matrix.dies.filter((d) => d.role === 'second')) {
    assert.equal(
      hostProvidesDrivers(
        { can_swap_host_driver: false, host_driver: row.drivers.host_driver },
        row,
      ),
      false,
    );
    const offers = [
      {
        id: 1,
        die: row.die,
        host_capabilities: { can_swap_host_driver: false, host_driver: row.drivers.host_driver },
      },
    ];
    assert.deepEqual(dieLegs(offers, { releaseWindowsRuns: 2 }, row), []);
  }
});
test('matrix: malformed driver objects return schema errors without throwing', () => {
  for (const value of [
    null,
    {},
    false,
    42,
    [],
    { host_driver: 12, linux_guest_driver: 12, windows_guest_driver: 12 },
  ]) {
    const m = copy();
    m.dies[0].drivers = value;
    assert.notDeepEqual(check(m), []);
  }
});

// Falsifier for a total validator: malformed JSON rows throw instead of reporting schema errors.
test('matrix: null and scalar rows report schema errors', () => {
  for (const row of [null, false, 0, 'TU106', []]) {
    const value = copy();
    value.dies[0] = row;
    assert.notDeepEqual(check(value), []);
  }
});
test('matrix: minimum versions compare numeric point components', () => {
  const value = copy();
  value.dies[2].minimum_drivers.host_driver = '580.95.05';
  assert.deepEqual(check(value), []);
  value.dies[2].minimum_drivers.host_driver = '580.178.04';
  assert.ok(check(value).some((e) => e.includes('minimum driver')));
});

// Falsifier: the version grammar accepts a final JavaScript line terminator.
test('driver version grammar rejects every trailing line terminator', async () => {
  const { driverVersion } = await import('../src/release-matrix.js');
  for (const tail of ['\n', '\r', '\u2028', '\u2029'])
    assert.equal(driverVersion('580.159.04' + tail), false);
});
test('matrix driver schema rejects consistent but line-terminated selections', () => {
  for (const tail of ['\r', '\u2028', '\u2029']) {
    const value = copy();
    value.dies[0].drivers.host_driver += tail;
    value.dies[1].cross_subrun.drivers.host_driver += tail;
    assert.ok(check(value).some((error) => error.includes('driver version')));
  }
});
