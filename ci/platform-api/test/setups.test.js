// GET /v1/ci/setups and /v1/ci/facets (migration 0003) on hand-written rows, READ-ONLY database.
// FALSIFIER: a PASS is counted for the wrong guest OS; a FAIL in the Windows stage of a run over
// both OSes is charged to the Linux setup (or the reverse); a claim (CPU vendor, driver) changes a
// verdict or a count; a filter value reaches SQL unvalidated; a running run is counted; a row tag,
// machine id, price, cost, CPU model or SPECviewperf datum appears in a response.
import test from 'node:test';
import assert from 'node:assert/strict';
import { api, seededDatabase } from './helpers.js';

const T = (n) => `2026-10-${String(10 + n).padStart(2, '0')}T12:00:00Z`;
let serial = 0;
const sha = (n) => String(n).repeat(40).slice(0, 40);
const tree = (n) => String(n).repeat(64).slice(0, 64);

// One ci_runs row plus its stages and claims. `claims` = {cpu, host, guests: {linux, windows}}.
function build(spec, rows) {
  const id = spec.id ?? 'syn-set-' + ++serial;
  const day = spec.day;
  const ended = spec.verdict === null ? null : T(day);
  rows.ci_runs.push({
    run_id: id,
    commit_sha: sha(spec.commit ?? 1),
    tree_hash: tree(spec.commit ?? 1),
    tree_kind: 'tree',
    tier: spec.tier ?? 'middle',
    profile: 'check',
    family: spec.family ?? 'ampere',
    die: spec.die,
    platform: spec.platform,
    verdict: spec.verdict,
    started_at: T(day),
    ended_at: ended,
    sort_at: ended ?? T(day),
    log_key: null,
    source: spec.source ?? 'reproduced',
    tag: 'secret-row-tag-' + id,
    recorded_at: T(day),
  });
  (spec.stages ?? []).forEach(([name, result], ord) =>
    rows.ci_run_stages.push({
      run_id: id,
      ord,
      name,
      rc: result === 'PASS' ? 0 : 1,
      duration_ms: 1,
      result,
    }),
  );
  rows.ci_host_facts.push({
    run_id: id,
    gpu_name: spec.gpu ?? 'NVIDIA GeForce RTX 3060',
    driver_versions_json: '{}',
  });
  rows.ci_runs_private.push({
    run_id: id,
    machine_id: 777000 + serial,
    price_usd_h: 0.31,
    cost_usd: 0.77,
  });
  if (spec.claims)
    for (const os of spec.platform === 'both' ? ['linux', 'windows'] : [spec.platform])
      rows.ci_run_claims.push({
        run_id: id,
        guest_os: os,
        guest_os_version: os === 'linux' ? 'ubuntu-24.04' : '11-ltsc-2024-26100',
        guest_driver: spec.claims.guests?.[os] ?? null,
        host_driver: spec.claims.host ?? null,
        cpu_vendor: spec.claims.cpu ?? null,
        cpu_model: spec.claims.cpu ? 'SECRET CPU MODEL' : null,
        cpu_arch: 'x86_64',
        claimed: 1,
      });
  return id;
}

const empty = () => ({
  ci_runs: [],
  ci_run_stages: [],
  ci_host_facts: [],
  ci_runs_private: [],
  ci_run_claims: [],
});
const rows = empty();
const g = (linux, windows) => ({ linux, windows });
const H1 = '580.95.05';
const H2 = '575.64.05';
const ids = {};
// GA106, both OSes, host H1, AMD: two PASS runs (days 1 and 3) and a FAIL in the Windows stage.
ids.p1 = build(
  {
    die: 'GA106',
    platform: 'both',
    verdict: 'PASS',
    day: 1,
    claims: { cpu: 'amd', host: H1, guests: g(H1, '580.88') },
    stages: [
      ['provision', 'PASS'],
      ['windows_gate', 'PASS'],
    ],
  },
  rows,
);
ids.p2 = build(
  {
    die: 'GA106',
    platform: 'both',
    verdict: 'PASS',
    day: 3,
    commit: 2,
    claims: { cpu: 'amd', host: H1, guests: g(H1, '580.88') },
    stages: [
      ['provision', 'PASS'],
      ['windows_gate', 'PASS'],
    ],
  },
  rows,
);
ids.winFail = build(
  {
    die: 'GA106',
    platform: 'both',
    verdict: 'FAIL',
    day: 5,
    commit: 3,
    claims: { cpu: 'amd', host: H1, guests: g(H1, '580.88') },
    stages: [
      ['provision', 'PASS'],
      ['windows_gate', 'FAIL'],
    ],
  },
  rows,
);
// A FAIL in a Linux stage of a run over both OSes: Linux FAIL, Windows UNATTRIBUTED.
ids.linFail = build(
  {
    die: 'GA106',
    platform: 'both',
    verdict: 'FAIL',
    day: 6,
    commit: 4,
    claims: { cpu: 'amd', host: H1, guests: g(H1, '580.88') },
    stages: [
      ['provision', 'PASS'],
      ['cuda_guest', 'FAIL'],
    ],
  },
  rows,
);
// A FAIL with no stage rows (a report limit): charged to both OSes, never to neither.
ids.bare = build(
  {
    die: 'GA106',
    platform: 'both',
    verdict: 'FAIL',
    day: 7,
    commit: 5,
    claims: { cpu: 'amd', host: H1, guests: g(H1, '580.88') },
  },
  rows,
);
// Intel, same host driver: its own group.
ids.intel = build(
  {
    die: 'GA106',
    platform: 'both',
    verdict: 'PASS',
    day: 4,
    commit: 6,
    claims: { cpu: 'intel', host: H1, guests: g(H1, '580.88') },
    stages: [
      ['provision', 'PASS'],
      ['windows_gate', 'PASS'],
    ],
  },
  rows,
);
// A Linux-only run on another host driver, a RETRY, an UNATTRIBUTED and a running run.
ids.old = build(
  {
    die: 'GA106',
    platform: 'linux',
    verdict: 'PASS',
    day: 2,
    commit: 7,
    tier: 'small',
    claims: { cpu: 'amd', host: H2, guests: g(H2) },
    stages: [['provision', 'PASS']],
  },
  rows,
);
ids.retry = build(
  {
    die: 'GA106',
    platform: 'linux',
    verdict: 'RETRY',
    day: 8,
    commit: 8,
    claims: { cpu: 'amd', host: H2, guests: g(H2) },
  },
  rows,
);
ids.unattr = build(
  {
    die: 'GA106',
    platform: 'linux',
    verdict: 'UNATTRIBUTED',
    day: 9,
    commit: 9,
    claims: { cpu: 'amd', host: H2, guests: g(H2) },
  },
  rows,
);
// Claims of a run still in flight are not a value anyone can filter by: arm and 590.00.01 occur only here.
ids.running = build(
  {
    die: 'GA106',
    platform: 'linux',
    verdict: null,
    day: 10,
    commit: 1,
    claims: { cpu: 'arm', host: '590.00.01', guests: g('590.00.01') },
  },
  rows,
);
// No claims at all (an older box): grouped under null CPU and drivers.
ids.noClaims = build(
  {
    die: 'GA106',
    platform: 'windows',
    verdict: 'PASS',
    day: 2,
    commit: 2,
    stages: [['windows_gate', 'PASS']],
  },
  rows,
);
// Another die, with a second GPU name reported.
ids.ga104 = build(
  {
    die: 'GA104',
    platform: 'linux',
    verdict: 'PASS',
    day: 11,
    commit: 3,
    gpu: 'NVIDIA GeForce RTX 3060 Ti',
    claims: { cpu: 'intel', host: H1, guests: g(H1) },
    stages: [['provision', 'PASS']],
  },
  rows,
);
ids.ga104b = build(
  {
    die: 'GA104',
    platform: 'linux',
    verdict: 'PASS',
    day: 12,
    commit: 4,
    gpu: 'NVIDIA GeForce RTX 3070',
    claims: { cpu: 'intel', host: H1, guests: g(H1) },
    stages: [['provision', 'PASS']],
  },
  rows,
);
ids.ad = build(
  {
    die: 'AD104',
    family: 'ada',
    platform: 'linux',
    verdict: 'FAIL',
    day: 13,
    commit: 5,
    claims: { cpu: 'other', host: H1, guests: g(H1) },
    stages: [['provision', 'FAIL']],
  },
  rows,
);

// A claim that is not a driver version (a hostile or broken box): shown as not reported, and never
// offered as a facet value.
ids.odd = build(
  {
    die: 'GB202',
    family: 'blackwell',
    platform: 'linux',
    verdict: 'PASS',
    day: 14,
    commit: 6,
    claims: { cpu: 'intel', host: '<b>evil</b>1.2', guests: g('<b>evil</b>1.2') },
    stages: [['provision', 'PASS']],
  },
  rows,
);

// Community rows (ci_runs.source 'reported' = unverified, 'reviewed'): reserved vocabulary, none are
// written today; they prove the filters and the per-row source for the later separate store.
ids.rep = build(
  {
    die: 'AD107',
    family: 'ada',
    platform: 'linux',
    verdict: 'PASS',
    day: 15,
    commit: 7,
    source: 'reported',
    claims: { cpu: 'amd', host: H1, guests: g(H1) },
    stages: [['provision', 'PASS']],
  },
  rows,
);
ids.rev = build(
  {
    die: 'AD107',
    family: 'ada',
    platform: 'linux',
    verdict: 'FAIL',
    day: 16,
    commit: 8,
    source: 'reviewed',
    claims: { cpu: 'amd', host: H1, guests: g(H1) },
    stages: [['provision', 'FAIL']],
  },
  rows,
);
ids.repro107 = build(
  {
    die: 'AD107',
    family: 'ada',
    platform: 'linux',
    verdict: 'PASS',
    day: 14,
    commit: 9,
    claims: { cpu: 'amd', host: H1, guests: g(H1) },
    stages: [['provision', 'PASS']],
  },
  rows,
);

const { call } = api({ database: seededDatabase({ rows }).reader });
const setups = async (query = '') => (await call('/v1/ci/setups' + query)).body;
const find = (list, os, die, cpu, host, guest) =>
  list.setups.find(
    (s) =>
      s.os === os &&
      s.die === die &&
      s.cpu_vendor === cpu &&
      s.host_driver === host &&
      s.guest_driver === guest,
  );

test('setups: grouped by OS, die, CPU vendor, host driver and guest driver, with counts', async () => {
  const body = await setups('?gpu=GA106');
  const lin = find(body, 'linux', 'GA106', 'amd', H1, H1);
  // Linux half of the AMD H1 group: 2 PASS, the Linux-stage FAIL, the stage-less FAIL; the FAIL in
  // the Windows stage of a run over both OSes is UNATTRIBUTED here.
  assert.deepEqual([lin.runs, lin.passed, lin.failed, lin.other], [5, 2, 2, 1]);
  const win = find(body, 'windows', 'GA106', 'amd', H1, '580.88');
  // Windows half: 2 PASS, the Windows-stage FAIL, the stage-less FAIL; the Linux-stage FAIL is
  // UNATTRIBUTED here.
  assert.deepEqual([win.runs, win.passed, win.failed, win.other], [5, 2, 2, 1]);
  assert.equal(lin.latest.id, ids.bare);
  assert.equal(lin.latest.result, 'FAIL');
  assert.equal(lin.latest.verdict, 'FAIL');
  assert.equal(lin.latest.commit, sha(5));
  assert.equal(lin.latest.tree, tree(5));
  assert.equal(lin.latest.ended_at, T(7));
  assert.equal(lin.last_pass.id, ids.p2);
  assert.equal(lin.last_pass.ended_at, T(3));
  assert.equal(win.latest.id, ids.bare);
  // Intel is its own group; the second host driver is its own group; no claims is its own group.
  assert.deepEqual(find(body, 'linux', 'GA106', 'intel', H1, H1)?.runs, 1);
  const old = find(body, 'linux', 'GA106', 'amd', H2, H2);
  assert.deepEqual([old.runs, old.passed, old.failed, old.other], [3, 1, 0, 2]);
  assert.equal(old.latest.result, 'UNATTRIBUTED');
  assert.equal(old.last_pass.id, ids.old);
  const none = find(body, 'windows', 'GA106', null, null, null);
  assert.deepEqual([none.runs, none.passed], [1, 1]);
  assert.equal(none.cpu_vendor, null);
  for (const s of body.setups) assert.equal(s.runs, s.passed + s.failed + s.other);
});

test('setups: a running run is never counted and a claim never changes a verdict', async () => {
  const body = await setups('?gpu=GA106');
  const total = body.setups.reduce((n, s) => n + s.runs, 0);
  // Every final GA106 run is counted once per OS it covers; the running one is not counted.
  const finalRuns = rows.ci_runs.filter((r) => r.die === 'GA106' && r.verdict !== null);
  const perOs = finalRuns.reduce((n, r) => n + (r.platform === 'both' ? 2 : 1), 0);
  assert.equal(total, perOs);
  assert.ok(!body.setups.some((s) => s.latest.id === ids.running));
  // RETRY and UNATTRIBUTED runs never add a pass.
  const passes = body.setups.reduce((n, s) => n + s.passed, 0);
  const wanted = finalRuns
    .filter((r) => r.verdict === 'PASS')
    .reduce((n, r) => n + (r.platform === 'both' ? 2 : 1), 0);
  assert.equal(passes, wanted);
});

test('setups: filters', async () => {
  const lin = await setups('?os=linux&gpu=GA106');
  assert.ok(lin.setups.every((s) => s.os === 'linux'));
  const two = await setups('?gpu=GA106,GA104');
  assert.deepEqual([...new Set(two.setups.map((s) => s.die))].sort(), ['GA104', 'GA106']);
  assert.deepEqual(two.filters.gpu, ['GA106', 'GA104']);
  const amd = await setups('?gpu=GA106&cpu=amd');
  assert.ok(amd.setups.length > 0 && amd.setups.every((s) => s.cpu_vendor === 'amd'));
  // A CPU filter hides the rows without claims (the page says so); it never matches null.
  assert.ok(!amd.setups.some((s) => s.cpu_vendor === null));
  const h2 = await setups('?gpu=GA106&driver=' + H2);
  assert.ok(
    h2.setups.length > 0 && h2.setups.every((s) => s.host_driver === H2 || s.guest_driver === H2),
  );
  const guest = await setups('?gpu=GA106&driver=580.88');
  assert.ok(guest.setups.length > 0 && guest.setups.every((s) => s.os === 'windows'));
  const small = await setups('?gpu=GA106&tier=small');
  assert.deepEqual(
    small.setups.map((s) => s.latest.tier),
    ['small'],
  );
  const other = await setups('?cpu=other');
  assert.deepEqual(
    other.setups.map((s) => s.die),
    ['AD104'],
  );
  assert.equal(other.setups[0].latest.result, 'FAIL');
  const all = await setups();
  assert.deepEqual([...new Set(all.setups.map((s) => s.die))].sort(), [
    'AD104',
    'AD107',
    'GA104',
    'GA106',
    'GB202',
  ]);
  const none = await setups('?gpu=GB207');
  assert.deepEqual(none.setups, []);
  assert.equal(none.truncated, false);
  const odd = await setups('?gpu=GB202');
  assert.deepEqual([odd.setups[0].host_driver, odd.setups[0].guest_driver], [null, null]);
  assert.ok(!JSON.stringify(odd).includes('evil'));
});

test('setups: limit truncates, newest first, and says so', async () => {
  const full = await setups('?gpu=GA106');
  assert.ok(full.setups.length > 3);
  const part = await setups('?gpu=GA106&limit=3');
  assert.equal(part.setups.length, 3);
  assert.equal(part.truncated, true);
  assert.deepEqual(part.setups, full.setups.slice(0, 3));
  for (let i = 1; i < full.setups.length; i++)
    assert.ok(full.setups[i - 1].last_at >= full.setups[i].last_at);
  assert.equal((await setups('?gpu=GA106&limit=100')).truncated, false);
});

test('setups: the response says which fields are claims and leaks nothing private', async () => {
  const privateText = [
    ...rows.ci_runs.map((r) => r.tag),
    'SECRET CPU MODEL',
    ...rows.ci_runs_private.map((p) => String(p.machine_id)),
    '0.31',
    '0.77',
  ];
  for (const path of ['/v1/ci/setups', '/v1/ci/setups?gpu=GA106', '/v1/ci/facets']) {
    const r = await call(path);
    assert.equal(r.status, 200, path);
    assert.equal(r.headers.get('access-control-allow-origin'), '*');
    assert.deepEqual(r.body.claimed, ['cpu_vendor', 'host_driver', 'guest_driver']);
    for (const value of privateText) assert.ok(!r.text.includes(value), path + ' ' + value);
    assert.doesNotMatch(r.text, /"(?:tag|machine_id|price_usd_h|cost_usd|cpu_model|content)"/);
    assert.doesNotMatch(r.text, /specviewperf|scores/i);
  }
  const post = await call('/v1/ci/setups', { method: 'POST', body: '{}' });
  assert.equal(post.status, 405);
  assert.equal(post.headers.get('access-control-allow-origin'), null);
  assert.equal((await call('/v1/ci/facets', { method: 'DELETE' })).status, 405);
});

test('setups: every filter is validated before any statement runs', async () => {
  for (const query of [
    '?os=macos',
    '?os=Linux',
    '?os=',
    '?gpu=ga106',
    '?gpu=GA106,',
    '?gpu=GA106,GA104,GA102,AD102,AD103',
    "?gpu=GA106'--",
    '?gpu=GA106%20OR%201',
    '?cpu=via',
    '?cpu=AMD',
    '?driver=580',
    '?driver=580.95.05.1',
    '?driver=%27',
    '?tier=huge',
    '?limit=0',
    '?limit=101',
    '?os=linux&os=windows',
    '?machine=1',
    '?price=1',
  ])
    assert.equal((await call('/v1/ci/setups' + query)).status, 400, query);
  assert.equal((await call('/v1/ci/facets?os=linux')).status, 400);
  assert.equal((await call('/v1/ci/setups/x')).status, 404);
});

test('facets: the values that exist among final runs', async () => {
  const { body } = await call('/v1/ci/facets');
  assert.deepEqual(body.os, [
    {
      value: 'linux',
      runs: rows.ci_runs.filter((r) => r.verdict !== null && r.platform !== 'windows').length,
    },
    {
      value: 'windows',
      runs: rows.ci_runs.filter((r) => r.verdict !== null && r.platform !== 'linux').length,
    },
  ]);
  assert.deepEqual(
    body.dies.map((d) => d.id),
    ['AD104', 'AD107', 'GA104', 'GA106', 'GB202'],
  );
  const ga104 = body.dies.find((d) => d.id === 'GA104');
  assert.deepEqual(ga104.gpu_names.map((n) => n.name).sort(), [
    'NVIDIA GeForce RTX 3060 Ti',
    'NVIDIA GeForce RTX 3070',
  ]);
  assert.deepEqual(body.cpu_vendors.map((v) => v.value).sort(), ['amd', 'intel', 'other']);
  assert.ok(!body.cpu_vendors.some((v) => v.value === 'arm'), 'a running run offers no value');
  assert.ok(!JSON.stringify(body).includes('590.00.01') && !JSON.stringify(body).includes('evil'));
  assert.deepEqual(
    body.cpu_archs.map((v) => v.value),
    ['x86_64'],
  );
  assert.deepEqual(body.host_drivers.map((v) => v.value).sort(), [H2, H1].sort());
  assert.deepEqual(body.guest_drivers.map((v) => v.value).sort(), [H2, H1, '580.88'].sort());
  assert.deepEqual(
    body.tiers.map((t) => t.value),
    ['middle', 'small'],
  );
  // The in-flight run is not a result: its commit-less claims are not offered as a value.
  assert.equal(
    body.dies.find((d) => d.id === 'GA106').runs,
    rows.ci_runs.filter((r) => r.die === 'GA106' && r.verdict !== null).length,
  );
});

test('setups and facets on an empty database are empty lists, not errors', async () => {
  const bare = api({ database: seededDatabase({ schemaOnly: true }).reader });
  const s = await bare.call('/v1/ci/setups?gpu=GA106');
  assert.equal(s.status, 200);
  assert.deepEqual(s.body.setups, []);
  const f = await bare.call('/v1/ci/facets');
  assert.equal(f.status, 200);
  for (const key of [
    'os',
    'dies',
    'cpu_vendors',
    'cpu_archs',
    'host_drivers',
    'guest_drivers',
    'tiers',
  ])
    assert.deepEqual(f.body[key], [], key);
});

test('setups: source and report_tier filters; every row carries its source', async () => {
  // Default: reproduced rows only. Community rows are opt-in.
  const base = await setups('?gpu=AD107');
  assert.deepEqual(
    base.setups.map((s) => [s.source, s.report_tier, s.runs]),
    [['reproduced', null, 1]],
  );
  assert.deepEqual(base.filters.source, ['reproduced']);
  assert.equal(base.setups[0].latest.id, ids.repro107);
  const both = await setups('?gpu=AD107&source=reproduced,community');
  assert.deepEqual(both.setups.map((s) => [s.source, s.report_tier]).sort(), [
    ['community', 'reviewed'],
    ['community', 'unverified'],
    ['reproduced', null],
  ]);
  // Rows of different sources are never merged into one group, even with identical drivers.
  assert.equal(both.setups.length, 3);
  const com = await setups('?gpu=AD107&source=community');
  assert.deepEqual(com.setups.map((s) => s.latest.id).sort(), [ids.rep, ids.rev].sort());
  assert.deepEqual(com.setups.map((s) => s.latest.result).sort(), ['FAIL', 'PASS']);
  const unv = await setups('?gpu=AD107&source=community&report_tier=unverified');
  assert.deepEqual(
    unv.setups.map((s) => [s.report_tier, s.latest.id]),
    [['unverified', ids.rep]],
  );
  const rev = await setups('?gpu=AD107&source=reproduced,community&report_tier=reviewed');
  assert.deepEqual(
    rev.setups.map((s) => s.report_tier),
    ['reviewed'],
  );
  // A report tier never matches a reproduced row; imported evidence is a static file, not a row.
  assert.deepEqual((await setups('?gpu=AD107&source=reproduced&report_tier=reviewed')).setups, []);
  assert.deepEqual((await setups('?gpu=AD107&source=imported')).setups, []);
  assert.deepEqual(
    (await setups('?gpu=AD107&source=imported,community')).setups.map((s) => s.source),
    ['community', 'community'],
  );
  assert.deepEqual((await setups('?source=imported')).setups, []);
  // Community rows never leak into the default view of the other dies either.
  assert.ok((await setups()).setups.every((s) => s.source === 'reproduced'));
});

test('setups: source and report_tier are validated', async () => {
  for (const query of [
    '?source=all',
    '?source=Reproduced',
    '?source=reproduced,',
    '?source=reproduced,community,imported,community',
    '?source=reviewed',
    '?report_tier=reported',
    '?report_tier=',
    "?source=community'--",
    '?source=community&source=reproduced',
  ])
    assert.equal((await call('/v1/ci/setups' + query)).status, 400, query);
});

test('facets: every filter lists the values that exist, including sources and report tiers', async () => {
  const { body } = await call('/v1/ci/facets');
  assert.deepEqual(
    body.sources.map((x) => x.value),
    ['reproduced', 'imported', 'community'],
  );
  const bySource = Object.fromEntries(body.sources.map((x) => [x.value, x]));
  assert.equal(bySource.reproduced.available, true);
  assert.equal(bySource.community.runs, 2);
  assert.equal(bySource.community.available, true);
  assert.equal(bySource.imported.available, false);
  assert.deepEqual(body.report_tiers, [
    { value: 'unverified', runs: 1 },
    { value: 'reviewed', runs: 1 },
  ]);
  for (const key of [
    'os',
    'dies',
    'cpu_vendors',
    'host_drivers',
    'guest_drivers',
    'tiers',
    'sources',
    'report_tiers',
  ])
    assert.ok(Array.isArray(body[key]), key);
  const bare = api({ database: seededDatabase({ schemaOnly: true }).reader });
  const empty = (await bare.call('/v1/ci/facets')).body;
  assert.deepEqual(empty.report_tiers, []);
  assert.equal(empty.sources.find((x) => x.value === 'community').available, false);
  assert.equal(empty.sources.find((x) => x.value === 'reproduced').available, false);
});
