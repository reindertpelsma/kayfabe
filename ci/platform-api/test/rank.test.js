import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { DIMS, DAY_MS, parseDriver, parseCpu, rank, compare } from '../src/rank.js';
import { DIE_ORDER, familyOfDie, diePlaces } from '../src/dies.js';
// Similarity ranking of subruns (src/rank.js; owner addendum 2026-11-07). FALSIFIER: an exact
// subrun is not first; a relaxation is not reported in match.relaxed (or an exact dimension is
// reported as relaxed); the default ladder order, a client's own `relax` order, `require` or
// `ignore` do not change the result the way they say; a die outside the family does not rank behind
// an in-family die at equal relaxation; two calls over the same rows (in any input order) return
// different lists; the die order table disagrees with the repository's die lists.
const T = Date.UTC(2026, 10, 1, 12);
let n = 0;
const row = (patch = {}) => ({
  run_id: 'vast-' + String(++n).padStart(3, '0') + '-1-0',
  sub: 'windows-580.88',
  kind: 'windows_guest',
  os: 'windows',
  die: 'GA106',
  family: 'ampere',
  tier: 'middle',
  commit_sha: 'a'.repeat(40),
  tree_hash: 't'.repeat(64),
  host_driver: '580.95.05',
  guest_driver: '580.88',
  cpu_vendor: 'amd',
  cpu_arch: 'x86_64',
  ended_at: new Date(T).toISOString().replace(/\.\d{3}Z$/, 'Z'),
  duration_ms: 1000,
  ...patch,
});
const WANT = {
  commit: 'a'.repeat(40),
  tree: 't'.repeat(64),
  die: 'GA106',
  os: 'windows',
  host_driver: '580.95.05',
  guest_driver: '580.88',
  cpu: { vendor: 'amd', arch: 'x86_64' },
  refTime: T,
};
const ALL = new Set(DIMS);
const run = (rows, wanted = WANT, options = {}) =>
  rank(rows, wanted, { active: ALL, relax: [...DIMS], ...options });
const dimsOf = (hit) => hit.match.relaxed.map((r) => r.dim);

test('the exact subrun is first and explains itself', () => {
  const near = row({ host_driver: '575.1.1' });
  const exact = row();
  const out = run([near, exact]);
  assert.equal(out[0].row, exact);
  assert.deepEqual(out[0].match.relaxed, []);
  assert.deepEqual(out[0].match.exact.sort(), [...DIMS].sort());
  assert.deepEqual(dimsOf(out[1]), ['host_driver']);
  assert.deepEqual(out[1].match.relaxed[0], {
    dim: 'host_driver',
    wanted: '580.95.05',
    found: '575.1.1',
  });
});

test('each ladder step relaxes exactly its own dimension', () => {
  const cases = {
    commit: [
      {
        commit_sha: 'b'.repeat(40),
        tree_hash: 'u'.repeat(64),
        ended_at: new Date(T - 3 * DAY_MS).toISOString().replace(/\.\d{3}Z$/, 'Z'),
      },
      ['commit'],
    ],
    cpu: [{ cpu_vendor: 'intel' }, ['cpu']],
    host_driver: [{ host_driver: '575.64.05' }, ['host_driver']],
    guest_driver: [{ guest_driver: '576.80' }, ['guest_driver']],
    die: [{ die: 'GA104' }, ['die']],
    os: [{ os: 'linux', sub: 'linux-580.95.05', guest_driver: '580.88' }, ['os']],
    family: [{ die: 'AD104', family: 'ada' }, ['die', 'family']],
  };
  for (const [dim, [patch, expected]] of Object.entries(cases)) {
    const out = run([row(patch)]);
    assert.deepEqual(dimsOf(out[0]), expected, dim);
  }
});

test('default ladder: with equal relaxed counts the more acceptable dimension ranks first', () => {
  const byDim = Object.fromEntries(
    Object.entries({
      commit: {
        commit_sha: 'b'.repeat(40),
        tree_hash: 'u'.repeat(64),
        ended_at: new Date(T - 3 * DAY_MS).toISOString().replace(/\.\d{3}Z$/, 'Z'),
      },
      cpu: { cpu_vendor: 'intel' },
      host_driver: { host_driver: '575.64.05' },
      guest_driver: { guest_driver: '576.80' },
      die: { die: 'GA104' },
      os: { os: 'linux', sub: 'linux-580.88' },
    }).map(([dim, patch]) => [dim, row(patch)]),
  );
  const shuffled = Object.values(byDim).reverse();
  const out = run(shuffled).map((h) => dimsOf(h)[0]);
  assert.deepEqual(out, ['commit', 'cpu', 'host_driver', 'guest_driver', 'die', 'os']);
});

test('fewer relaxed dimensions beat a more acceptable single step', () => {
  const two = row({ cpu_vendor: 'intel', host_driver: '575.1.1' });
  const one = row({ os: 'linux', sub: 'linux-580.88' });
  const out = run([two, one]);
  assert.equal(out[0].row, one, 'one relaxed dimension (os) before two (cpu, host_driver)');
});

test('a custom relax order reorders the ladder', () => {
  const driver = row({ host_driver: '575.64.05' });
  const cpu = row({ cpu_vendor: 'intel' });
  assert.deepEqual(
    run([driver, cpu]).map((h) => h.row),
    [cpu, driver],
  );
  const custom = run([cpu, driver], WANT, { relax: ['host_driver', 'cpu'] });
  assert.deepEqual(
    custom.map((h) => h.row),
    [driver, cpu],
  );
});

test('a dimension that is not in relax must match exactly (and require and ignore say so)', () => {
  const rows = [row({ cpu_vendor: 'intel' }), row({ host_driver: '575.1.1' }), row()];
  const only = run(rows, WANT, { relax: ['cpu'] });
  assert.deepEqual(only.map(dimsOf), [[], ['cpu']]);
  const none = run(rows, WANT, { relax: [] });
  assert.equal(none.length, 1);
  assert.equal(none[0].row, rows[2]);
  const ignore = run(rows, WANT, {
    active: new Set(DIMS.filter((d) => d !== 'cpu')),
    relax: ['host_driver'],
  });
  assert.deepEqual(ignore.map(dimsOf), [[], [], ['host_driver']]);
  assert.ok(
    ignore[0].match.exact.every((d) => d !== 'cpu'),
    'an ignored dimension is neither exact nor relaxed',
  );
});

test('a die outside the family ranks behind an in-family die at equal relaxation', () => {
  const sameFamily = row({
    die: 'GA102',
    host_driver: '575.1.1',
    ended_at: new Date(T - DAY_MS * 9).toISOString().replace(/\.\d{3}Z$/, 'Z'),
  });
  const wrong = row({ die: 'AD106', family: 'ada', host_driver: '575.1.1' });
  const out = run([wrong, sameFamily]);
  assert.equal(out[0].row, sameFamily);
  assert.deepEqual(dimsOf(out[1]), ['host_driver', 'die', 'family']);
  // Without the family step the other family is not offered at all.
  const strict = run([wrong, sameFamily], WANT, { relax: DIMS.filter((d) => d !== 'family') });
  assert.deepEqual(
    strict.map((h) => h.row),
    [sameFamily],
  );
  // Nor when the die itself is not relaxable.
  const exactDie = run([wrong, sameFamily], WANT, { relax: ['host_driver', 'family'] });
  assert.deepEqual(
    exactDie.map((h) => h.row),
    [],
  );
});

test('die proximity inside a family is the number of places in the order table', () => {
  const rows = ['GA102', 'GA104', 'GA107'].map((die) => row({ die }));
  const out = run(rows, { ...WANT, die: 'GA106' });
  assert.deepEqual(
    out.map((h) => h.row.die),
    ['GA104', 'GA107', 'GA102'].sort(
      (a, b) => diePlaces('GA106', a) - diePlaces('GA106', b) || (a < b ? -1 : 1),
    ),
  );
  assert.equal(diePlaces('GA106', 'GA104'), 1);
  assert.equal(diePlaces('GA106', 'GA102'), 3);
  assert.equal(diePlaces('GA106', 'AD106'), null);
  assert.equal(diePlaces('GA106', 'CC_86'), null);
  assert.equal(diePlaces('TU116', 'TU117'), 1);
});

test('commit dimension: same tree, then the same day, then older or newer by distance', () => {
  const day = (ms) => new Date(T + ms).toISOString().replace(/\.\d{3}Z$/, 'Z');
  const other = { commit_sha: 'b'.repeat(40), tree_hash: 'u'.repeat(64) };
  const sameTree = row({ commit_sha: 'c'.repeat(40) });
  const sameDay = row({ ...other, ended_at: day(-5 * 3600 * 1000) });
  const older = row({ ...other, ended_at: day(-3 * DAY_MS) });
  const newer = row({ ...other, ended_at: day(+10 * DAY_MS) });
  const ancient = row({ ...other, ended_at: day(-40 * DAY_MS) });
  const out = run([ancient, newer, older, sameDay, sameTree]);
  assert.deepEqual(
    out.map((h) => h.row),
    [sameTree, sameDay, older, newer, ancient],
  );
  assert.deepEqual(out[0].match.relaxed, []);
  assert.equal(out[1].match.relaxed[0].note, 'same_day_older');
  assert.equal(out[2].match.relaxed[0].note, 'older');
  assert.equal(out[3].match.relaxed[0].note, 'newer');
  assert.equal(out[2].match.relaxed[0].found, 'b'.repeat(40));
  // The commit alone (no tree) is exact too.
  assert.deepEqual(
    run([row({ tree_hash: 'z'.repeat(64) })], { ...WANT, tree: null })[0].match.relaxed,
    [],
  );
  // Nothing known about the wanted commit: the newest candidate is the reference.
  const unknown = run([older, newer], {
    ...WANT,
    tree: 'q'.repeat(64),
    commit: null,
    refTime: null,
  });
  assert.equal(unknown[0].row, newer);
  assert.equal(unknown[0].match.relaxed[0].note, 'reference_unknown');
});

test('cpu: same vendor and arch, then same arch, then other or unknown', () => {
  const same = row();
  const other = row({ cpu_vendor: 'arm', cpu_arch: 'aarch64' });
  const unknown = row({ cpu_vendor: null, cpu_arch: null });
  const arch = row({ cpu_vendor: 'intel' }); // created last: its run id sorts after the others'
  const out = run([unknown, other, arch, same]);
  assert.equal(out[0].row, same);
  assert.equal(out[1].row, arch, 'same architecture, another vendor');
  assert.deepEqual(
    out
      .slice(2)
      .map((h) => h.row)
      .sort((x, y) => (x.run_id < y.run_id ? -1 : 1)),
    [other, unknown],
  );
  assert.deepEqual(out[1].match.relaxed[0], {
    dim: 'cpu',
    wanted: 'amd-x86_64',
    found: 'intel-x86_64',
  });
  assert.equal(unknown.cpu_vendor, null);
  assert.equal(out.find((h) => h.row === unknown).match.relaxed[0].found, null);
  assert.deepEqual(parseCpu('amd'), { vendor: 'amd', arch: 'x86_64' });
  assert.deepEqual(parseCpu('arm'), { vendor: 'arm', arch: 'aarch64' });
  assert.deepEqual(parseCpu('intel-aarch64'), { vendor: 'intel', arch: 'aarch64' });
  assert.equal(parseCpu('sparc'), null);
});

test('drivers: same, same branch, numeric distance; unknown last', () => {
  const rows = ['580.95.05', '580.159.03', '575.64.05', '535.1.1', null].map((host_driver) =>
    row({ host_driver }),
  );
  const out = run(rows, { ...WANT, host_driver: '580.159.03' });
  assert.deepEqual(
    out.map((h) => h.row.host_driver),
    ['580.159.03', '580.95.05', '575.64.05', '535.1.1', null],
  );
  assert.equal(out[4].match.relaxed[0].found, null);
  assert.deepEqual(parseDriver('580.95.05'), [580, 95, 5]);
  assert.deepEqual(parseDriver('580.88'), [580, 88, 0]);
  assert.equal(parseDriver('580'), null);
  assert.equal(parseDriver(null), null);
});

test('min_closeness caps the number of relaxed dimensions', () => {
  const rows = [
    row(),
    row({ cpu_vendor: 'intel' }),
    row({ cpu_vendor: 'intel', host_driver: '575.1.1' }),
  ];
  assert.equal(run(rows, WANT, { maxRelaxed: 0 }).length, 1);
  assert.equal(run(rows, WANT, { maxRelaxed: 1 }).length, 2);
  assert.equal(run(rows, WANT, { maxRelaxed: 7 }).length, 3);
});

test('deterministic: any input order gives one list; ties break on recency, run id, subrun id', () => {
  const a = row({ run_id: 'vast-050-1-0', host_driver: '575.1.1' });
  const b = row({ run_id: 'vast-040-1-0', host_driver: '575.1.1' });
  const c = row({ run_id: 'vast-040-1-0', sub: 'windows-580.89', host_driver: '575.1.1' });
  const d = row({
    run_id: 'vast-060-1-0',
    host_driver: '575.1.1',
    ended_at: new Date(T + 1000).toISOString().replace(/\.\d{3}Z$/, 'Z'),
  });
  const orders = [
    [a, b, c, d],
    [d, c, b, a],
    [c, a, d, b],
    [b, d, a, c],
  ];
  const lists = orders.map((rows) => run(rows).map((h) => h.row.run_id + '/' + h.row.sub));
  for (const l of lists) assert.deepEqual(l, lists[0]);
  assert.deepEqual(lists[0], [
    'vast-060-1-0/windows-580.88',
    'vast-040-1-0/windows-580.88',
    'vast-040-1-0/windows-580.89',
    'vast-050-1-0/windows-580.88',
  ]);
});

test('sort modes: recent and duration reorder, closeness is the default', () => {
  const old = row({
    ended_at: new Date(T - DAY_MS).toISOString().replace(/\.\d{3}Z$/, 'Z'),
    duration_ms: 10,
  });
  const fresh = row({ host_driver: '575.1.1', duration_ms: 99999 });
  const none = row({ host_driver: '575.2.2', duration_ms: null });
  // closeness: the exact one, then the nearer driver (575.2.2 is nearer 580.95.05 than 575.1.1)
  assert.deepEqual(
    run([fresh, old, none]).map((h) => h.row),
    [old, none, fresh],
  );
  const recent = run([old, fresh, none], WANT, { sort: 'recent' }).map((h) => h.row);
  assert.equal(recent.at(-1), old, 'the oldest is last whatever its closeness');
  assert.equal(recent.length, 3);
  assert.deepEqual(
    run([fresh, old, none], WANT, { sort: 'duration' }).map((h) => h.row),
    [old, fresh, none],
  );
});

test('with nothing wanted every subrun is a hit, newest first, nothing relaxed', () => {
  const rows = [row({ ended_at: new Date(T - 9).toISOString().replace(/\.\d{3}Z$/, 'Z') }), row()];
  const out = rank(rows, {}, { active: ALL, relax: [...DIMS] });
  assert.equal(out.length, 2);
  assert.ok(out.every((h) => h.match.relaxed.length === 0 && h.match.exact.length === 0));
});

test('compare never reports a dimension twice and never relaxes an unwanted one', () => {
  const { exact, relaxed } = compare(row({ os: 'linux', sub: 'linux-x' }), { os: 'windows' }, ALL, {
    newest: T,
  });
  assert.deepEqual(exact, []);
  assert.deepEqual(
    relaxed.map((r) => r.dim),
    ['os'],
  );
});

test('dies.js agrees with the repository die lists', () => {
  const defaults = JSON.parse(
    readFileSync(new URL('../../vast-broker/defaults.json', import.meta.url), 'utf8'),
  );
  const known = new Set(Object.values(DIE_ORDER).flat());
  for (const rule of defaults.dieRules)
    assert.ok(known.has(rule.die), rule.die + ' (dieRules) is missing from the order table');
  for (const die of defaults.sweepDies.filter((d) => /^[A-Z]{2}[0-9]{3}$/.test(d)))
    assert.ok(known.has(die), die);
  for (const [family, dies] of Object.entries(DIE_ORDER)) {
    assert.ok(Object.hasOwn(defaults.families, family) || family === 'hopper', family);
    for (const die of dies) assert.equal(familyOfDie(die), family, die);
    assert.equal(new Set(dies).size, dies.length);
  }
  // The corrected Turing rules: TU117 is the smallest, TU102 the largest.
  assert.deepEqual(DIE_ORDER.turing, ['TU117', 'TU116', 'TU106', 'TU104', 'TU102']);
  assert.equal(familyOfDie('CC_89'), 'ada');
  assert.equal(familyOfDie('XX999'), 'unknown');
  assert.equal(familyOfDie(null), 'unknown');
});
