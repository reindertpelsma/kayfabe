// Falsifier: the Worker can put a KF_* variable into a box's environment that the box's contract
// (scripts/ci/vast_report.py SPEC) drops or refuses. 2026-11-10 (PR #334): KF_LINUX_APPS=1 was sent
// but had no SPEC row, so load_provider_env and write_env_file removed it and the box skipped the
// Linux apps stage silently. The box side is read from vast_report.py itself (env_contract_check.py);
// there is no second list to drift. Local mocks only: no rental, no network.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { setup, rentBody, sha } from './helpers.js';
import { tierEnvironment } from '../src/tier.js';
import { RELEASE_MATRIX, releaseEnvironment } from '../src/release-matrix.js';

const read = (name) => readFileSync(new URL('../src/' + name, import.meta.url), 'utf8');
const contract = (input) => {
  const run = spawnSync(
    'python3',
    ['-B', '-I', new URL('./env_contract_check.py', import.meta.url).pathname],
    { input: JSON.stringify(input), encoding: 'utf8', timeout: 15000 },
  );
  assert.equal(run.status, 0, run.stderr);
  return JSON.parse(run.stdout);
};
// Every environment tierEnvironment can return: each profile, tier name and option, with the
// release matrix rows. Combinations the function refuses throw and are skipped.
function everyTierEnvironment() {
  const found = [];
  const rows = [undefined, ...RELEASE_MATRIX.dies];
  for (const profile of ['check', 'sweep', 'release'])
    for (const tierName of [undefined, 'small', 'middle', 'large', 'adhoc'])
      for (const windowsRuns of [1, 2])
        for (const stages of [[], ['specviewperf'], ['llm_parity']])
          for (const releaseMatrix of rows)
            for (const spec of [undefined, 'lanea', 'laneb', 'both'])
              for (const winApps of [undefined, 'small', 'mid', 'full'])
                for (const linuxApps of [undefined, true])
                  for (const rehearsal of [undefined, true])
                    try {
                      found.push(
                        tierEnvironment(profile, {
                          tierName,
                          windowsRuns,
                          stages,
                          releaseMatrix,
                          spec,
                          winApps,
                          linuxApps,
                          rehearsal,
                        }),
                      );
                    } catch {
                      // refused by design
                    }
  return found;
}
const NAMES = [
  'KF_RELEASE_REHEARSAL',
  'KF_MERGE_WINDOWS',
  'KF_TIER',
  'KF_WINDOWS_RUNS',
  'KF_STAGES',
  'KF_SPEC',
  'KF_WINAPPS',
  'KF_LINUX_APPS',
  ...Object.keys(releaseEnvironment(RELEASE_MATRIX.dies[0])),
  'KF_WINGATE_GUEST_DRIVER',
];

test('the enumeration reaches every key tierEnvironment can emit', () => {
  const keys = new Set(everyTierEnvironment().flatMap((e) => Object.keys(e)));
  for (const name of NAMES) assert.ok(keys.has(name), name + ' is never emitted: enumerator blind');
});
test('every key and value tierEnvironment can emit passes the box contract', () => {
  const all = everyTierEnvironment();
  assert.ok(all.length > 100);
  const pairs = [...new Set(all.flatMap((e) => Object.entries(e).map((p) => JSON.stringify(p))))];
  const verdict = contract({ pairs: pairs.map((p) => JSON.parse(p)) });
  assert.deepEqual(verdict.unknown, []);
  assert.deepEqual(verdict.malformed, []);
});
test('every KF_ name tier.js and release-matrix.js mention has a SPEC row', () => {
  const mentioned = new Set(
    ['tier.js', 'release-matrix.js'].flatMap((name) => read(name).match(/\bKF_[A-Z_]+\b/g) ?? []),
  );
  const { spec } = contract({ env: {} });
  for (const name of mentioned) assert.ok(spec.includes(name), name + ' has no SPEC row');
});
test('every KF_ name in the rental environment block has a SPEC row', () => {
  const text = read('index.js');
  const start = text.indexOf('const environment = {');
  const end = text.indexOf('r.id = await vast.create(', start);
  assert.ok(start > 0 && end > start, 'rental environment block not found');
  const names = new Set(text.slice(start, end).match(/\bKF_[A-Z_]+\b/g));
  assert.ok(names.size >= 15, 'block scan found too few names');
  const { spec } = contract({ env: {} });
  for (const name of names) assert.ok(spec.includes(name), name + ' has no SPEC row');
});
test('the Linux apps opt-in is accepted as "1"', () => {
  assert.deepEqual(contract({ env: { KF_LINUX_APPS: '1' } }).malformed, []);
});
test('a falsy Linux apps opt-in is refused, not read as off', () => {
  for (const bad of ['0', '', ' 0'])
    assert.deepEqual(contract({ env: { KF_LINUX_APPS: bad } }).malformed, ['KF_LINUX_APPS'], bad);
});
test('a truthy-word or padded Linux apps opt-in is refused', () => {
  for (const bad of ['true', 'yes', '11', ' 1', '1\n'])
    assert.deepEqual(contract({ env: { KF_LINUX_APPS: bad } }).malformed, ['KF_LINUX_APPS'], bad);
});
test('the whole environment of a mocked signed middle rental passes the box contract', async () => {
  const h = await setup({ MIDDLE_APPS_ENABLED: undefined });
  const body = rentBody(sha, { linuxApps: true, winApps: 'mid' });
  const response = await h.call('/rent', { body });
  assert.equal(response.status, 200, await response.clone().text());
  try {
    const env = h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url)).body.env;
    assert.equal(env.KF_LINUX_APPS, '1');
    const verdict = contract({ env, full: true });
    assert.deepEqual(verdict.unknown, []);
    assert.deepEqual(verdict.problems, []);
  } finally {
    await h.call('/stop/123-1');
  }
});
// Regression 2026-11-11: KF_LINUX_APPS was dropped between tier.js and the box. The emission side:
// the signed middle tier's request, taken from middleAppsRequest exactly as index.js builds it, must
// put both opt-ins into the environment; every other tier/profile must carry neither.
// Killers: (1) the exact-value asserts below, (2) the SPEC membership of each emitted name.
import { middleAppsRequest } from '../src/tier.js';
// Release shape v2 (2026-11-12): a release MIDDLE leg (the second die of a family) asks for the apps pair too.
test('a release middle leg carries the apps pair; a release small or adhoc leg does not', () => {
  assert.deepEqual(middleAppsRequest('release', 'middle', true), {
    linuxApps: true,
    winApps: 'mid',
  });
  assert.deepEqual(middleAppsRequest('release', 'middle', false), {});
  assert.deepEqual(middleAppsRequest('release', 'small', true), {});
  assert.deepEqual(middleAppsRequest('release', 'adhoc', true), {});
});
test('the middle and large tier requests carry KF_LINUX_APPS=1 and KF_WINAPPS=mid, and only they', () => {
  const env = tierEnvironment('check', {
    tierName: 'middle',
    ...middleAppsRequest('check', 'middle', true),
  });
  assert.equal(env.KF_LINUX_APPS, '1');
  assert.equal(env.KF_WINAPPS, 'mid');
  const { spec } = contract({ env: {} });
  for (const name of ['KF_LINUX_APPS', 'KF_WINAPPS']) assert.ok(spec.includes(name), name);
  assert.deepEqual(contract({ pairs: Object.entries(env) }).malformed, []);
  for (const [profile, tierName, enabled] of [
    ['check', 'middle', false],
    ['check', 'small', true],
    ['check', 'adhoc', true],
    ['sweep', 'middle', true],
    ['release', 'small', true],
    ['check', 'large', true],
    ['sweep', 'large', true],
  ]) {
    const e = tierEnvironment(profile, {
      tierName,
      ...middleAppsRequest(profile, tierName, enabled),
    });
    assert.ok(
      !('KF_LINUX_APPS' in e) && !('KF_WINAPPS' in e),
      profile + '/' + tierName + '/' + enabled,
    );
  }
});

// 2026-11-12 (release rehearsal, owner: large includes middle). FALSIFIER: a large leg that does not send the apps
// pair, a rehearsal marker outside the large release profile or beside a matrix row, or a marker the box contract drops.
test('a large leg carries the middle apps pair; a rehearsal leg adds exactly KF_RELEASE_REHEARSAL=1', () => {
  const apps = middleAppsRequest('release', 'large', true);
  assert.deepEqual(apps, { linuxApps: true, winApps: 'mid' });
  assert.deepEqual(middleAppsRequest('release', 'large', false), {});
  const env = tierEnvironment('release', {
    tierName: 'large',
    windowsRuns: 2,
    rehearsal: true,
    ...apps,
  });
  assert.equal(env.KF_LINUX_APPS, '1');
  assert.equal(env.KF_WINAPPS, 'mid');
  assert.equal(env.KF_RELEASE_REHEARSAL, '1');
  assert.equal(env.KF_TIER, 'large');
  assert.ok(!('KF_HOST_DRIVER' in env) && !('KF_RELEASE_SUBRUNS' in env));
  assert.deepEqual(contract({ pairs: Object.entries(env) }).malformed, []);
  assert.deepEqual(contract({ pairs: Object.entries(env) }).unknown, []);
  for (const bad of ['0', 'true', '', ' 1'])
    assert.deepEqual(
      contract({ env: { KF_RELEASE_REHEARSAL: bad } }).malformed,
      ['KF_RELEASE_REHEARSAL'],
      bad,
    );
});
test('the rehearsal marker is refused outside large/release and beside a matrix row', () => {
  for (const [profile, tierName] of [
    ['check', 'middle'],
    ['check', 'small'],
    ['sweep', 'adhoc'],
    ['release', 'middle'],
    ['release', undefined],
  ])
    assert.throws(() => tierEnvironment(profile, { tierName, rehearsal: true }), /rehearsal/);
  assert.throws(
    () =>
      tierEnvironment('release', {
        tierName: 'large',
        rehearsal: true,
        releaseMatrix: RELEASE_MATRIX.dies[0],
      }),
    /rehearsal/,
  );
  assert.throws(() => tierEnvironment('release', { tierName: 'large', rehearsal: 1 }), /rehearsal/);
  assert.ok(!('KF_RELEASE_REHEARSAL' in tierEnvironment('release', { tierName: 'large' })));
});
