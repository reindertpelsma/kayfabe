// SPDX-License-Identifier: Apache-2.0 OR GPL-2.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { validateTable, selectDrivers, exactVersionKey } from './validate.js';

const checked = '2026-11-08T00:00:00Z';
const url = (os, v) => os === 'linux'
  ? `https://download.nvidia.com/XFree86/Linux-x86_64/${v}/NVIDIA-Linux-x86_64-${v}.run`
  : `https://us.download.nvidia.com/Windows/${v}/${v}-desktop-win10-win11-64bit-international-dch-whql.exe`;
function driver(os, version, branch = 580, last_tested = null) {
  return { os, version, branch, url: url(os, version), size: 12345,
    ...(os === 'windows' ? { branch_evidence: { branch, version, source_url: 'https://www.nvidia.com/en-us/drivers/' } } : {}),
    sha256: 'a'.repeat(64), status: 'PINNED', release_date: null,
    probe: { method: 'HEAD', status: 200, checked_at: checked, final_url: url(os, version) },
    checksum_source: { kind: os === 'linux' ? 'nvidia-sha256' : 'download-sha256',
      url: url(os, version) + (os === 'linux' ? '.sha256sum' : ''), checked_at: checked },
    last_tested, test_records: [], discovery_source_urls: ['https://www.nvidia.com/en-us/drivers/'] };
}
function fixture() {
  return { schema_version: 1, status: 'DESIGN-ONLY', generated_at: checked,
    families: Object.fromEntries(['turing', 'ampere', 'ada', 'blackwell'].map(f => [f, {
      minimum_supported_branch: f === 'blackwell' ? 570 : 'UNKNOWN',
      sweep_floor: f === 'blackwell' ? 570 : 535,
      source_url: 'https://www.nvidia.com/en-us/drivers/', checked_at: checked,
      evidence: 'Fixture only; no hardware claim',
    }])),
    branches: [{ id: 580, name: 'R580', branch_type: 'LTS',
      source_url: 'https://docs.nvidia.com/datacenter/tesla/drivers/releases.json',
      first_version: { linux: '580.1.01', windows: '580.11' },
      last_version: { linux: '580.1.08', windows: '580.18' },
      repo_status: 'never-tested', test_records: [] }],
    known_bad: [], core_reference_versions: [],
    abi_boundary_hypothesis: { status: 'HYPOTHESIS', falsifier: 'A different layout inside a group',
      used_for_selection: false }, sources: [],
    drivers: [...Array.from({ length: 8 }, (_, i) => driver('linux', `580.1.0${i + 1}`)),
      ...Array.from({ length: 8 }, (_, i) => driver('windows', `580.${i + 11}`))] };
}
const box = (id = 'a', family = 'ada', role = 'flagship') => ({ id, family, role });
const versions = plan => plan.flatMap(b => b.runs.flatMap(r => Object.values(r.drivers).map(d => d.version)));

test('fixture passes schema validation', () => assert.equal(validateTable(fixture()), true));
test('committed catalog passes validation', () => {
  if (process.env.DRV1_FIXTURES_ONLY) return;
  assert.equal(validateTable(JSON.parse(readFileSync(new URL('../driver-availability.json', import.meta.url)))), true);
});
test('same seed returns the same complete assignments', () => {
  const t = fixture(), boxes = [box('b'), box('a')];
  const p = selectDrivers('release-1', boxes, t);
  assert.deepEqual(p, selectDrivers('release-1', boxes, t));
  assert.equal(p.length, 2);
  assert.equal(p[0].runs[0].drivers.host_driver.os, 'linux');
  assert.equal(p[0].runs[0].drivers.linux_guest_driver.os, 'linux');
  assert.equal(p[0].runs[0].drivers.windows_guest_driver.os, 'windows');
});
test('canonical input ordering is independent of array order', () => {
  const t = fixture(), p = selectDrivers('s', [box('b'), box('a')], t);
  t.drivers.reverse(); t.branches.reverse();
  assert.deepEqual(p, selectDrivers('s', [box('a'), box('b')], t));
});
test('seed changes tie choices across a bank of seeds', () => {
  const plans = new Set(Array.from({ length: 20 }, (_, i) => JSON.stringify(versions(selectDrivers(`${i}`, [box()], fixture())))));
  assert.ok(plans.size > 1);
});
test('seed participates even when box IDs are different', () => {
  const plans = new Set(Array.from({ length: 20 }, (_, i) => JSON.stringify(versions(selectDrivers(`${i}`, [box('other')], fixture())))));
  assert.ok(plans.size > 1);
});
test('no exact version repeats across boxes or axes', () => {
  const v = versions(selectDrivers('s', [box('a'), box('b'), box('c')], fixture())).map(exactVersionKey);
  assert.equal(new Set(v).size, v.length);
});
test('exhaustion never falls back to a version on another box', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('windows', '580.11')];
  assert.throws(() => selectDrivers('s', [box('a'), box('b')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
for (const [name, edit] of [
  ['UNPINNED', d => { d.status = 'UNPINNED:needs-download'; d.sha256 = null; d.checksum_source = null; }],
  ['404', d => { d.probe.status = 404; d.status = 'UNAVAILABLE'; d.sha256 = null; d.size = null; d.checksum_source = null; }],
  ['403', d => { d.probe.status = 403; d.status = 'UNKNOWN:probe-error'; d.sha256 = null; d.checksum_source = null; }],
]) {
  for (const axis of ['linux', 'windows']) test(`${name} ${axis} is never selected`, () => {
    const t = fixture(); t.drivers.filter(d => d.os === axis).slice(0, 7).forEach(edit);
    if (axis === 'linux') assert.throws(() => selectDrivers('s', [box()], t), /INSUFFICIENT_PINNED_VERSIONS/);
    else assert.equal(selectDrivers('s', [box()], t)[0].runs[0].drivers.windows_guest_driver.version, '580.18');
  });
}
for (const axis of ['linux', 'windows']) test(`known_bad excludes ${axis} even with a valid pin`, () => {
  const t = fixture();
  t.known_bad = t.drivers.filter(d => d.os === axis).slice(0, 7).map(d => ({ os: d.os, version: d.version, reason: 'fixture' }));
  if (axis === 'linux') assert.throws(() => selectDrivers('s', [box()], t), /INSUFFICIENT_PINNED_VERSIONS/);
  else assert.equal(selectDrivers('s', [box()], t)[0].runs[0].drivers.windows_guest_driver.version, '580.18');
});
test('never tested versions rank before dated tests', () => {
  const t = fixture(); t.drivers.forEach(d => { d.last_tested = '2026-11-01'; });
  t.drivers[0].last_tested = null; t.drivers[1].last_tested = null; t.drivers[8].last_tested = null;
  assert.deepEqual(versions(selectDrivers('s', [box()], t)).sort(), ['580.1.01', '580.1.02', '580.11'].sort());
});
test('oldest test dates rank before newer dates for every OS', () => {
  const t = fixture(); t.drivers.forEach(d => { d.last_tested = '2026-11-01'; });
  t.drivers[0].last_tested = '2020-01-01'; t.drivers[1].last_tested = '2020-01-02'; t.drivers[8].last_tested = '2020-01-01';
  assert.deepEqual(versions(selectDrivers('s', [box()], t)).sort(), ['580.1.01', '580.1.02', '580.11'].sort());
});
test('Blackwell excludes branches below its floor', () => {
  const t = fixture(); t.branches[0].id = 550; t.branches[0].name = 'R550';
  t.drivers = t.drivers.map(d => driver(d.os, d.version.replace('580', '550'), 550));
  assert.throws(() => selectDrivers('s', [box('a', 'blackwell')], t), /NO_SUPPORTED_BRANCHES/);
});
test('unsupported Blackwell branches are explicit in coverage accounting', () => {
  const t = fixture();
  t.branches.push({ ...t.branches[0], id: 550, name: 'R550', first_version: { linux: null, windows: null }, last_version: { linux: null, windows: null } });
  const p = selectDrivers('s', [box('a', 'blackwell')], t);
  assert.deepEqual(p[0].excluded_branches, [{ branch: 550, reason: 'BELOW_FAMILY_FLOOR' }]);
  assert.deepEqual(p[0].runs.map(r => r.branch), [580]);
});
test('each supported branch remains in the sweep', () => {
  const t = fixture();
  t.branches.push({ ...t.branches[0], id: 590, name: 'R590', first_version: { linux: '590.1', windows: '591.1' }, last_version: { linux: '590.2', windows: '591.1' } });
  t.drivers.push(driver('linux', '590.1', 590), driver('linux', '590.2', 590), driver('windows', '591.1', 590));
  assert.deepEqual(selectDrivers('s', [box()], t)[0].runs.map(r => r.branch), [580, 590]);
});
test('missing pinned Windows for a branch cannot produce a partial sweep', () => {
  const t = fixture(); t.drivers = t.drivers.filter(d => d.os === 'linux');
  assert.throws(() => selectDrivers('s', [box()], t), /INSUFFICIENT_PINNED_VERSIONS.*windows/);
});
test('a second branch without pins cannot silently disappear', () => {
  const t = fixture();
  t.branches.push({ ...t.branches[0], id: 590, name: 'R590', first_version: { linux: null, windows: null }, last_version: { linux: null, windows: null } });
  assert.throws(() => selectDrivers('s', [box()], t), /INSUFFICIENT_PINNED_VERSIONS.*590/);
});
test('matching preserves a scarce cross-OS exact version', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.11'), driver('linux', '580.12'), driver('linux', '580.13'), driver('windows', '580.11')];
  const p = selectDrivers('s', [box()], t);
  assert.equal(new Set(versions(p).map(exactVersionKey)).size, 3);
});
test('one normalized version on two OSes cannot fill both guest axes', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.11.0'), driver('linux', '580.12'), driver('windows', '580.11')];
  assert.throws(() => selectDrivers('s', [box()], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('exact version ownership spans all branch runs', () => {
  const t = fixture();
  t.branches.push({ ...t.branches[0], id: 570, name: 'R570' });
  t.drivers.forEach(d => { d.last_tested = '2026-11-01'; });
  t.drivers[0].last_tested = null;
  t.drivers.push(driver('linux', '570.1', 570), driver('linux', '570.2', 570), driver('windows', '580.1.01', 570));
  const v = versions(selectDrivers('s', [box()], t)).map(exactVersionKey);
  assert.equal(new Set(v).size, v.length);
});
test('cross-branch OS overlap cannot bypass exhaustion', () => {
  const t = fixture();
  t.branches.push({ ...t.branches[0], id: 570, name: 'R570' });
  t.drivers = [driver('linux', '570.1', 570), driver('linux', '570.2', 570), driver('windows', '580.1', 570),
    driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  assert.throws(() => selectDrivers('s', [box()], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('explicit family core reference can be shared by core boxes', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  t.core_reference_versions = t.drivers.map(d => ({ family: 'ada', os: d.os, version: d.version }));
  const p = selectDrivers('s', [box('a', 'ada', 'core-reference'), box('b', 'ada', 'core-reference')], t);
  assert.equal(versions(p).length, 6);
});
test('core exception cannot cross family boundary', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  t.core_reference_versions = t.drivers.map(d => ({ family: 'ada', os: d.os, version: d.version }));
  assert.throws(() => selectDrivers('s', [box('a', 'ada', 'core-reference'), box('b', 'ampere', 'core-reference')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('ordinary roles cannot use the core sharing exception', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  t.core_reference_versions = t.drivers.map(d => ({ family: 'ada', os: d.os, version: d.version }));
  assert.throws(() => selectDrivers('s', [box('a'), box('b')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('one ordinary box cannot share with a core-reference box', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  t.core_reference_versions = t.drivers.map(d => ({ family: 'ada', os: d.os, version: d.version }));
  assert.throws(() => selectDrivers('s', [box('a', 'ada', 'core-reference'), box('b')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('a designation in each family still cannot share across families', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  t.core_reference_versions = ['ada', 'ampere'].flatMap(family => t.drivers.map(d => ({ family, os: d.os, version: d.version })));
  assert.throws(() => selectDrivers('s', [box('a', 'ada', 'core-reference'), box('b', 'ampere', 'core-reference')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('GeForce boxes cannot select an RTX Enterprise package', () => {
  const t = fixture(); t.drivers.filter(d => d.os === 'windows').forEach(d => { d.product_line = 'rtx-enterprise'; });
  assert.throws(() => selectDrivers('s', [box()], t), /INSUFFICIENT_PINNED_VERSIONS.*windows/);
});
test('explicit enterprise boxes cannot select a GeForce package', () => {
  assert.throws(() => selectDrivers('s', [{ ...box(), product_line: 'rtx-enterprise' }], fixture()), /INSUFFICIENT_PINNED_VERSIONS.*windows/);
});
test('core designation does not permit within-box duplicate axes', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('windows', '580.11')];
  t.core_reference_versions = [{ family: 'ada', os: 'linux', version: '580.1' }];
  assert.throws(() => selectDrivers('s', [box('a', 'ada', 'core-reference')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('selector has no input mutation', () => {
  const t = fixture(), boxes = [box('b'), box('a')], before = JSON.stringify({ t, boxes });
  selectDrivers('s', boxes, t); assert.equal(JSON.stringify({ t, boxes }), before);
});
test('deep-frozen inputs are accepted', () => {
  function freeze(o) { if (o && typeof o === 'object') { Object.values(o).forEach(freeze); Object.freeze(o); } return o; }
  assert.equal(selectDrivers('s', freeze([box('b'), box('a')]), freeze(fixture())).length, 2);
});
for (const [label, edit] of [
  ['schema version', t => { t.schema_version = 2; }],
  ['malformed sha256', t => { t.drivers[0].sha256 = 'bad'; }],
  ['missing size', t => { t.drivers[0].size = null; }],
  ['unofficial URL', t => { t.drivers[0].url = 'https://example.com/driver.run'; }],
  ['credential URL', t => { t.drivers[0].url = t.drivers[0].url.replace('https://', 'https://user:pass@'); }],
  ['redirect outside NVIDIA', t => { t.drivers[0].probe.final_url = 'https://example.com/file'; }],
  ['missing checksum provenance', t => { t.drivers[0].checksum_source = null; }],
  ['invalid test date', t => { t.drivers[0].last_tested = '2026-02-30'; }],
  ['future test date', t => { t.drivers[0].last_tested = '2027-01-01'; }],
  ['GET instead of HEAD', t => { t.drivers[0].probe.method = 'GET'; }],
  ['PINNED with HTTP404', t => { t.drivers[0].probe.status = 404; }],
  ['PINNED with HTTP403', t => { t.drivers[0].probe.status = 403; }],
  ['Linux version in wrong branch', t => { t.drivers[0].branch = 590; t.branches.push({ ...t.branches[0], id: 590, name: 'R590' }); }],
  ['notebook-only Windows installer', t => { t.drivers[8].url = t.drivers[8].url.replace('-desktop-', '-notebook-'); }],
  ['duplicate normalized version', t => { t.drivers.push(driver('linux', '580.1.1')); }],
  ['missing known-good proof', t => { t.branches[0].repo_status = 'known-good'; }],
  ['ABI used for selection', t => { t.abi_boundary_hypothesis.used_for_selection = true; }],
]) {
  test(`schema rejects ${label}`, () => { const t = fixture(); edit(t); assert.throws(() => validateTable(t)); });
  test(`selector rejects ${label} before assignment`, () => { const t = fixture(); edit(t); assert.throws(() => selectDrivers('s', [box()], t)); });
}
for (const boxes of [[box('a'), box('a')], [box('a', 'unknown')], [box('', 'ada')], [box('a', 'ada', '')]]) {
  test(`invalid boxes rejected: ${JSON.stringify(boxes)}`, () => assert.throws(() => selectDrivers('s', boxes, fixture())));
}
test('version keys equate omitted and zero patch', () => assert.equal(exactVersionKey('580.88'), exactVersionKey('580.088.0')));
test('version keys equate zero padding', () => assert.equal(exactVersionKey('580.01.01'), exactVersionKey('580.1.1')));
test('Windows 576.02 can explicitly belong to R570', () => {
  const t = fixture(); t.branches.push({ ...t.branches[0], id: 570, name: 'R570' }, { ...t.branches[0], id: 575, name: 'R575' });
  t.drivers.push(driver('windows', '576.02', 570));
  assert.equal(validateTable(t), true);
});
test('Windows branch mapping is never inferred from a numeric interval', () => {
  const t = fixture(); t.branches.push({ ...t.branches[0], id: 570, name: 'R570' }, { ...t.branches[0], id: 575, name: 'R575' });
  t.drivers.push(driver('linux', '570.1', 570), driver('linux', '570.2', 570), driver('windows', '576.02', 570));
  t.drivers.push(driver('linux', '575.1', 575), driver('linux', '575.2', 575), driver('windows', '576.28', 575));
  assert.equal(selectDrivers('s', [box()], t)[0].runs[0].drivers.windows_guest_driver.version, '576.02');
});
for (const edit of [d => { delete d.branch_evidence; }, d => { d.branch_evidence.branch = 570; }]) {
  test(`Windows branch evidence rejected: ${edit}`, () => { const t = fixture(); edit(t.drivers[8]); assert.throws(() => validateTable(t)); });
  test(`selector refuses bad Windows branch evidence: ${edit}`, () => { const t = fixture(); edit(t.drivers[8]); assert.throws(() => selectDrivers('s', [box()], t), /Windows branch/); });
}
test('core roles without a designated version cannot share', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  assert.throws(() => selectDrivers('s', [box('a', 'ada', 'core-reference'), box('b', 'ada', 'core-reference')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('a core reference for another version cannot be reused', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11'), driver('windows', '580.12')];
  t.core_reference_versions = [{ family: 'ada', os: 'windows', version: '580.12' }];
  assert.throws(() => selectDrivers('s', [box('a', 'ada', 'core-reference'), box('b', 'ada', 'core-reference')], t), /INSUFFICIENT_PINNED_VERSIONS/);
});
test('core sharing still assigns three distinct axes on each box', () => {
  const t = fixture(); t.drivers = [driver('linux', '580.1'), driver('linux', '580.2'), driver('windows', '580.11')];
  t.core_reference_versions = t.drivers.map(d => ({ family: 'ada', os: d.os, version: d.version }));
  for (const b of selectDrivers('s', [box('a', 'ada', 'core-reference'), box('b', 'ada', 'core-reference')], t)) {
    assert.equal(new Set(versions([b]).map(exactVersionKey)).size, 3);
  }
});
test('small pools agree with an exhaustive independent assignment oracle', () => {
  const norm = v => [...v.split('.').map(Number), 0].slice(0, 3).join('.');
  let cases = 0;
  for (const coreRoles of [false, true, 'mixed']) for (const sameFamily of [false, true]) {
    for (const linuxCount of [2, 3, 4]) for (const windowsCount of [1, 2]) for (const referenceMode of [0, 1, 2]) {
      const t = fixture();
      t.drivers = [...Array.from({ length: linuxCount }, (_, i) => driver('linux', `580.${i + 1}`)),
        ...Array.from({ length: windowsCount }, (_, i) => driver('windows', `580.${i + 11}`))];
      const boxes = [box('a', 'ada', coreRoles ? 'core-reference' : 'flagship'),
        box('b', sameFamily ? 'ada' : 'ampere', coreRoles === true ? 'core-reference' : 'second')];
      t.core_reference_versions = referenceMode === 0 ? [] : t.drivers.filter(d => referenceMode === 2 || d.os === 'linux')
        .flatMap(d => ['ada', 'ampere'].map(family => ({ family, os: d.os, version: d.version })));
      const slots = boxes.flatMap(b => ['linux', 'linux', 'windows'].map(os => ({ box: b, os })));
      const chosen = [];
      function permitted(slot, d) {
        return chosen.every(([prior, pd]) => norm(pd.version) !== norm(d.version)
          || prior.box.id !== slot.box.id && prior.os === slot.os && prior.box.family === slot.box.family
          && prior.box.role === 'core-reference' && slot.box.role === 'core-reference'
          && t.core_reference_versions.some(r => r.family === slot.box.family && r.os === d.os && norm(r.version) === norm(d.version)));
      }
      function brute(i) {
        if (i === slots.length) return true;
        for (const d of t.drivers.filter(d => d.os === slots[i].os)) {
          if (!permitted(slots[i], d)) continue;
          chosen.push([slots[i], d]);
          if (brute(i + 1)) return true;
          chosen.pop();
        }
        return false;
      }
      const feasible = brute(0); chosen.length = 0;
      let plan;
      try { plan = selectDrivers('oracle', boxes, t); }
      catch (e) { assert.match(e.message, /INSUFFICIENT_PINNED_VERSIONS/); }
      assert.equal(Boolean(plan), feasible, `feasibility case ${cases}`);
      if (plan) for (const b of plan) for (const d of Object.values(b.runs[0].drivers)) {
        const slot = { box: b, os: d.os };
        assert.equal(permitted(slot, d), true, `soundness case ${cases}`);
        chosen.push([slot, d]);
      }
      cases++;
    }
  }
  assert.equal(cases, 108);
});

export { fixture, driver, box, versions };
