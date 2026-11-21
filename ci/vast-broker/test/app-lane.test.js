// The app lane planner (src/app-lane.js, app-lane.json; docs/OWNER_RULINGS.md section AZ, docs/design/V3_APP_LANE_PLANNER.md).
// GPU-free, no network, nothing rented, nothing deployed.
// FALSIFIER (written before the code): a core-path change is reported as not required (or a docs, site, ci, test-only or
// excluded-crate change as required); the committed core_paths differ from the section AZ item 5 list; a glob lets `*` cross a
// directory or `**` stop at one; a leg is pinned to a die or an image is named in planner code; an image's leg is priced at
// the release profile's flat bandwidth instead of its own measured bandwidth; an offer with too little VRAM or disk is chosen; a dearer
// offer is chosen over a cheaper one that qualifies; the first release misses an image/family pair; a rotation leaves an
// image/family pair unrun for more than four releases or an image out of a release; an estimate over the daily cap is admitted.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { offlineAppPlan, releaseConfig } from '../release-plan.mjs';
import { estimateBandwidth, plannedStages } from '../src/bandwidth.js';
import { offerPrice, resourceCaps } from '../src/policy.js';
import {
  APP_LANE,
  validateAppLaneConfig,
  appLaneRequired,
  selectRotation,
  recordRotation,
  imageProfile,
  appLegTotal,
} from '../src/app-lane.js';

const here = (p) => new URL(p, import.meta.url);
const market = () =>
  JSON.parse(readFileSync(here('./fixtures/release-market-synthetic.json'))).offers;
const VARS = { RELEASE_ENABLED: 'true', MAX_CONCURRENT_RENTALS: '9' };
// The section AZ item 5 list, written out independently of app-lane.json.
const AZ_INCLUDE = [
  'crates/kf-*/src/**',
  'crates/kf-chip/**',
  'cuda/walk/**',
  'qemu/hw/misc/kf3/**',
  'broker/**',
  'firmware/**',
  'scripts/bench/windows/propimage.py',
  'scripts/bench/windows/clocknet_assert.py',
];
const AZ_EXCLUDE = [
  'crates/kf-harness/**',
  'crates/kf-rawclient/**',
  'crates/kf-trace/**',
  'crates/*/tests/**',
];
const CORE = APP_LANE.core_paths;
const req = (...files) => appLaneRequired(files, CORE);

// ---- (1) core_paths as data, and appLaneRequired -------------------------------------------------------------------------
test('core_paths is exactly the section AZ item 5 list, as data', () => {
  assert.ok(Array.isArray(CORE));
  assert.deepEqual(
    CORE.filter((g) => !g.startsWith('!')),
    AZ_INCLUDE,
  );
  assert.deepEqual(
    CORE.filter((g) => g.startsWith('!')).map((g) => g.slice(1)),
    AZ_EXCLUDE,
  );
  assert.deepEqual(validateAppLaneConfig(APP_LANE), []);
});
test('each core rule matches, and reports the matching path', () => {
  for (const file of [
    'crates/kf-chip/src/lib.rs',
    'crates/kf-chan/src/generated/tables.rs',
    'crates/kf-linux-raw/src/a/b/c.rs',
    'crates/kf-chip/Cargo.toml',
    'crates/kf-chip/data/families.json',
    'cuda/walk/walk.cu',
    'cuda/walk/ptx/walk.ptx',
    'qemu/hw/misc/kf3/kf3.c',
    'broker/main.c',
    'broker/sub/dir/x.h',
    'firmware/gop/driver.c',
    'scripts/bench/windows/propimage.py',
    'scripts/bench/windows/clocknet_assert.py',
  ]) {
    const r = req('docs/x.md', file);
    assert.equal(r.required, true, file);
    assert.deepEqual(r.matches, [file], file);
  }
});
test('paths that are not core do not require the app lane', () => {
  for (const file of [
    'docs/design/V3_X.md',
    'site/index.html',
    'ci/vast-broker/src/policy.js',
    'scripts/ci/doc_freshness.py',
    'scripts/bench/windows/other.py',
    'scripts/bench/windows/propimage.pyc',
    'scripts/bench/windows/sub/propimage.py',
    'traces/run1/README.md',
    'README.md',
    'crates/kf-chan/Cargo.toml', // not under src/ and not kf-chip
    'crates/kf-chan/benches/b.rs',
    'crates/kayfabe-rm-ladder/src/main.rs', // the frozen pre-v3 tree is not kf-*
    'cuda/walker/x.cu', // a sibling, not cuda/walk/
    'broker2/x.c',
    'xbroker/x.c',
    'firmware',
  ]) {
    const r = req(file);
    assert.equal(r.required, false, file);
    assert.deepEqual(r.matches, [], file);
  }
});
test('the excluded crates are not core, even under src/', () => {
  for (const crate of ['kf-harness', 'kf-rawclient', 'kf-trace']) {
    const r = req('crates/' + crate + '/src/lib.rs', 'crates/' + crate + '/Cargo.toml');
    assert.equal(r.required, false, crate);
    assert.deepEqual(r.excluded, ['crates/' + crate + '/src/lib.rs']);
  }
  // a crate whose name only starts like an excluded one stays core
  assert.equal(req('crates/kf-harness2/src/lib.rs').required, true);
  assert.equal(req('crates/kf-tracer/src/lib.rs').required, true);
});
test('test-only paths are not core, also inside kf-chip', () => {
  for (const file of [
    'crates/kf-chan/tests/t.rs',
    'crates/kf-chip/tests/x.rs',
    'crates/kf-chip/tests/data/golden/a.json',
  ])
    assert.equal(req(file).required, false, file);
  // tests/ inside src/ is still src
  assert.equal(req('crates/kf-chan/src/tests/mod.rs').required, true);
  // one core file among test-only ones is enough, and only it is named
  const r = req('crates/kf-chan/tests/t.rs', 'cuda/walk/walk.cu', 'docs/a.md');
  assert.equal(r.required, true);
  assert.deepEqual(r.matches, ['cuda/walk/walk.cu']);
  assert.deepEqual(r.excluded, []); // kf-chan/tests matches no core pattern at all
  assert.deepEqual(req('crates/kf-chip/tests/x.rs').excluded, ['crates/kf-chip/tests/x.rs']);
});
test('glob semantics: * stays inside a segment, ** crosses segments', () => {
  const m = (globs, file) => appLaneRequired([file], globs).required;
  assert.equal(m(['a/*/c'], 'a/b/c'), true);
  assert.equal(m(['a/*/c'], 'a/b/x/c'), false);
  assert.equal(m(['a/**'], 'a/b/x/c'), true);
  assert.equal(m(['a/**'], 'a'), false);
  assert.equal(m(['a/**/c'], 'a/c'), true);
  assert.equal(m(['a/**/c'], 'a/b/x/c'), true);
  assert.equal(m(['kf-*'], 'kf-chip'), true);
  assert.equal(m(['kf-*'], 'kf-chip/src'), false);
  assert.equal(m(['a.b'], 'axb'), false); // dot is literal
  assert.equal(m(['a+b'], 'a+b'), true);
  assert.equal(m(['a/**', '!a/x/**'], 'a/x/y'), false);
  assert.equal(m(['a/**', '!a/x/**'], 'a/z/y'), true);
});
test('input is normalised and bad input is refused', () => {
  assert.equal(req('./cuda/walk/walk.cu').required, true);
  assert.deepEqual(req('./cuda/walk/walk.cu').matches, ['cuda/walk/walk.cu']);
  assert.equal(req().required, false);
  for (const bad of ['../cuda/walk/x', '/cuda/walk/x', 'cuda\\walk\\x', '', 'a/../cuda/walk/x', 7])
    assert.throws(() => req(bad), /changed file/, String(bad));
  assert.throws(() => appLaneRequired('cuda/walk/x', CORE), /array/);
  assert.throws(() => appLaneRequired([], []), /core_paths/);
  for (const bad of ['a/[b]', 'a/{b,c}', 'a/?', '', '!', '/a/**', 'a/../b', 'a\\b'])
    assert.ok(validateAppLaneConfig({ ...APP_LANE, core_paths: [bad] }).length, bad);
});
test('planner code and its data never name a specific image outside the config', () => {
  const code = [
    readFileSync(here('../src/app-lane.js'), 'utf8'),
    readFileSync(here('../release-plan.mjs'), 'utf8'),
  ].join('\n');
  for (const img of APP_LANE.images) assert.ok(!code.includes(img.id), img.id);
  assert.ok(!/propimage_apps|\bimg[a-z]\b|image [A-D]\b/.test(code));
});

// ---- (2) the app leg kind ------------------------------------------------------------------------------------------------
const IMAGES = APP_LANE.images.map((i) => i.id);
const ids = (plan) => plan.legs.map((l) => l.image + '/' + l.family);
test('config: images carry their needs and their stages are stages of bandwidth.json', () => {
  assert.ok(APP_LANE.images.length >= 1);
  const bw = JSON.parse(readFileSync(here('../bandwidth.json')));
  for (const img of APP_LANE.images) {
    assert.ok(Number.isInteger(img.disk_gb) && img.disk_gb > 0, img.id);
    assert.ok(Number.isFinite(img.vram_gb) && img.vram_gb > 0, img.id);
    assert.equal(typeof img.manual_only, 'boolean', img.id);
    assert.ok(img.stages.length);
    for (const s of img.stages) assert.ok(bw.stages[s], img.id + ':' + s);
  }
  const bad = structuredClone(APP_LANE);
  bad.images[0].stages = ['no_such_stage'];
  assert.ok(validateAppLaneConfig(bad).length);
  const dup = structuredClone(APP_LANE);
  dup.images.push(structuredClone(dup.images[0]));
  assert.ok(validateAppLaneConfig(dup).length);
});
test('image profile: bandwidth is the measured GB of the image stages, not the release profile', () => {
  for (const img of APP_LANE.images) {
    const p = imageProfile(img);
    const instruction = { tier: 'large', windows: false, propimages: [{ label: img.id }] };
    const only = img.stages.filter((s) =>
      plannedStages(instruction, { only: img.stages }).includes(s),
    );
    const e = estimateBandwidth(instruction, 'large', { only });
    assert.equal(p.bandwidth_down_gb, e.down_gb);
    assert.ok(
      p.bandwidth_up_gb >= e.up_gb,
      'a stage without downloads of its own still uploads its evidence',
    );
    assert.ok(
      p.bandwidth_down_gb > 40 &&
        p.bandwidth_down_gb < resourceCaps(releaseConfig(VARS), 'release').bandwidthDownGb,
      'measured, below the release profile',
    );
    assert.equal(p.disk_gb, img.disk_gb);
    assert.equal(p.vram_gb, img.vram_gb);
  }
});
test('leg price is compute+storage over the hard limit plus bandwidth priced per measured GB', async () => {
  const plan = await offlineAppPlan(market(), VARS, { dailyCapUsd: 1000, firstRelease: true });
  assert.ok(plan.legs.length);
  const { hard_ms, cron_ms } = plan.cost_model;
  assert.equal(hard_ms, APP_LANE.roles.image.hard_minutes * 60000);
  for (const leg of plan.legs) {
    const offer = market().find((o) => o.id === leg.offer_id);
    const p = imageProfile(APP_LANE.images.find((i) => i.id === leg.image));
    const want =
      ((offer.dph_base + (offer.storage_cost * p.disk_gb) / 720) * (hard_ms + cron_ms)) / 3600000 +
      p.bandwidth_down_gb * offer.inet_down_cost +
      p.bandwidth_up_gb * offer.inet_up_cost;
    assert.ok(Math.abs(leg.est_usd - want) < 1e-3, leg.image + ' ' + leg.est_usd + ' ' + want);
    assert.equal(leg.bandwidth_down_gb, p.bandwidth_down_gb);
    assert.equal(leg.bandwidth_up_gb, p.bandwidth_up_gb);
    assert.ok(
      Math.abs(leg.est_usd - appLegTotal(offer, p, { hard: hard_ms, cronPeriod: cron_ms })) < 1e-3,
    );
  }
});
test('appLegTotal equals the release offerPrice total when given the release profile numbers', () => {
  const cfg = releaseConfig(VARS);
  // The numbers come from the release (large) resource profile in defaults.json, not from literals: a change to the profile
  // (130 -> 135 GB on 2026-11-17) must not break the pin, while a change to the formula in either function still does.
  const r = resourceCaps(cfg, 'release');
  assert.ok(r.diskGb > 0 && r.bandwidthDownGb > 0);
  for (const offer of market()) {
    const release = offerPrice(offer, cfg, 'release');
    const mine = appLegTotal(
      offer,
      {
        disk_gb: r.diskGb,
        bandwidth_down_gb: r.bandwidthDownGb,
        bandwidth_up_gb: r.bandwidthUpGb,
      },
      { hard: cfg.hard, cronPeriod: cfg.cronPeriod },
    );
    assert.ok(Math.abs(release.total - mine) < 1e-9, String(offer.id));
  }
});
test('a leg is not pinned to a die: the cheapest qualifying offer of the family is taken, any die', async () => {
  const plan = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    images: [IMAGES[0]],
    families: ['ada', 'blackwell'],
  });
  const byFamily = Object.fromEntries(plan.legs.map((l) => [l.family, l]));
  // synthetic market: ada = RTX 4070 SUPER (7005, 7006) and RTX 4090 (7007, 7008)
  assert.ok(['AD104', 'AD102'].includes(byFamily.ada.die));
  assert.equal(
    byFamily.ada.offer_id,
    7005,
    'cheapest ada offer, a die the release matrix calls the second die',
  );
  // blackwell: 7009 is the cheapest RTX 5070
  assert.equal(byFamily.blackwell.offer_id, 7009);
  for (const leg of plan.legs) assert.ok(!('release_matrix' in leg) && !('drivers' in leg));
});
test('an offer below the image VRAM or disk need is never chosen', async () => {
  const m = market();
  m.find((o) => o.id === 7005).gpu_ram = 4096; // cheapest ada, too small
  m.find((o) => o.id === 7006).disk_space = 400; // fine
  const plan = await offlineAppPlan(m, VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    images: [IMAGES[0]],
    families: ['ada'],
  });
  assert.equal(plan.legs[0].offer_id, 7006);
  const vram = APP_LANE.images[0].vram_gb;
  for (const o of m) o.gpu_ram = vram * 1024 - 1;
  const none = await offlineAppPlan(m, VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    images: [IMAGES[0]],
    families: ['ada'],
  });
  assert.equal(none.legs.length, 0);
  assert.deepEqual(none.unplaced, [{ image: IMAGES[0], family: 'ada', reason: 'NO_OFFER' }]);
  assert.ok(none.refusals.includes('INCOMPLETE'));
  assert.equal(none.admitted, false);
});
test('inter-leg: two legs never share a machine, and a family without offers is reported', async () => {
  const plan = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    families: ['ampere'],
  });
  const machines = plan.legs.map((l) => l.machine_id);
  assert.equal(new Set(machines).size, machines.length);
  const some = market().filter((o) => o.compute_cap !== 860);
  const none = await offlineAppPlan(some, VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    families: ['ampere'],
  });
  assert.equal(none.legs.length, 0);
  assert.equal(none.unplaced.length, IMAGES.length);
});
test('manual-only images carry their --only stages into the leg', async () => {
  const plan = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    families: ['ada'],
    images: [IMAGES[0]],
  });
  const img = APP_LANE.images[0];
  assert.equal(plan.legs[0].manual_only, img.manual_only);
  assert.deepEqual(plan.legs[0].only, img.manual_only ? img.stages : undefined);
});

// ---- (3) first release, rotation, daily cap -------------------------------------------------------------------------------
const FAMILIES = ['turing', 'ampere', 'ada', 'blackwell'];
test('first release: every image on every family of the matrix', async () => {
  const plan = await offlineAppPlan(market(), VARS, { dailyCapUsd: 1000, firstRelease: true });
  assert.equal(plan.mode, 'first_release');
  assert.deepEqual(plan.families, FAMILIES);
  assert.deepEqual(
    ids(plan).sort(),
    IMAGES.flatMap((i) => FAMILIES.map((f) => i + '/' + f)).sort(),
  );
  assert.equal(plan.admitted, true, plan.refusals.join());
  const sum = plan.legs.reduce((s, l) => s + l.est_usd, 0);
  assert.ok(Math.abs(plan.estimate_usd - sum) < 1e-3);
  assert.equal(plan.legs.length, IMAGES.length * FAMILIES.length);
});
test('first release ignores the rotation table', async () => {
  const rotation = {
    release_seq: 5,
    last: Object.fromEntries(IMAGES.map((i) => [i, { turing: 4 }])),
  };
  const plan = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    rotation,
  });
  assert.equal(plan.legs.length, IMAGES.length * FAMILIES.length);
});
test('rotation: oldest first, never-run before run, every image in every release', () => {
  const pick = (table, seq) =>
    selectRotation({
      images: ['i1', 'i2'],
      families: FAMILIES,
      table,
      releaseSeq: seq,
      windowReleases: 4,
    });
  const first = pick({}, 1);
  assert.equal(first.length, 2);
  assert.deepEqual(new Set(first.map((p) => p.image)), new Set(['i1', 'i2']));
  const table = {
    release_seq: 1,
    last: {
      i1: { turing: 1, ampere: 1, ada: 0, blackwell: 0 },
      i2: { turing: 0, ampere: 0, ada: 1, blackwell: 1 },
    },
  };
  const second = pick(table, 2);
  // oldest are i1/ada, i1/blackwell, i2/turing, i2/ampere (age 2); two slots, one per image first
  assert.deepEqual(second.map((p) => p.image + '/' + p.family).sort(), ['i1/ada', 'i2/turing']);
});
for (const [images, families] of [
  [['a', 'b'], FAMILIES],
  [['a', 'b', 'c', 'd'], FAMILIES],
  [
    ['a', 'b', 'c'],
    ['x', 'y', 'z'],
  ],
  [['a'], FAMILIES],
  [
    ['a', 'b', 'c', 'd', 'e', 'f', 'g'],
    ['f1', 'f2', 'f3', 'f4', 'f5'],
  ],
])
  test(
    'rotation guarantee: ' +
      images.length +
      ' images x ' +
      families.length +
      ' families, gap <= 4 releases',
    () => {
      let table = {};
      const last = new Map();
      const W = 4;
      for (let seq = 1; seq <= 40; seq++) {
        const picked = selectRotation({
          images,
          families,
          table,
          releaseSeq: seq,
          windowReleases: W,
        });
        assert.deepEqual(
          new Set(picked.map((p) => p.image)),
          new Set(images),
          'every image runs in release ' + seq,
        );
        assert.equal(new Set(picked.map((p) => p.image + '/' + p.family)).size, picked.length);
        for (const p of picked) {
          const k = p.image + '/' + p.family;
          if (last.has(k)) assert.ok(seq - last.get(k) <= W, k + ' waited ' + (seq - last.get(k)));
          last.set(k, seq);
        }
        table = recordRotation(table, picked, seq);
        // from the fourth release on, every pair has met
        if (seq >= W)
          for (const i of images)
            for (const f of families)
              assert.ok(last.has(i + '/' + f), i + '/' + f + ' unmet at ' + seq);
      }
    },
  );
test('rotation survives a stale table: an overdue pair is forced in', () => {
  const table = { release_seq: 9, last: { a: { x: 1, y: 8 }, b: { x: 8, y: 8 } } };
  const picked = selectRotation({
    images: ['a', 'b'],
    families: ['x', 'y'],
    table,
    releaseSeq: 10,
    windowReleases: 4,
  });
  assert.ok(picked.some((p) => p.image === 'a' && p.family === 'x'));
});
test('rotation planning through the offers: fewer legs than the first release, and the table is updated', async () => {
  const plan = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: 1000,
    rotation: { release_seq: 0, last: {} },
  });
  assert.equal(plan.mode, 'rotation');
  assert.equal(plan.release_seq, 1);
  assert.ok(
    plan.legs.length >= IMAGES.length && plan.legs.length < IMAGES.length * FAMILIES.length,
  );
  const after = recordRotation({ release_seq: 0, last: {} }, plan.legs, 1);
  for (const l of plan.legs) assert.equal(after.last[l.image][l.family], 1);
  assert.equal(after.release_seq, 1);
  // without a table and without firstRelease the planner refuses to guess
  await assert.rejects(offlineAppPlan(market(), VARS, { dailyCapUsd: 1000 }), /rotation/);
});
test('an unplaced leg is not recorded as run', () => {
  const t = recordRotation({}, [{ image: 'a', family: 'x' }], 3);
  assert.deepEqual(t, { release_seq: 3, last: { a: { x: 3 } } });
  // recording never mutates its input
  const before = { release_seq: 1, last: { a: { x: 1 } } };
  recordRotation(before, [{ image: 'a', family: 'y' }], 2);
  assert.deepEqual(before, { release_seq: 1, last: { a: { x: 1 } } });
});
test('estimate over the daily cap is a hard refusal; so is a missing cap', async () => {
  const open = await offlineAppPlan(market(), VARS, { dailyCapUsd: 1000, firstRelease: true });
  const total = open.estimate_usd;
  assert.ok(total > 0);
  const under = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: total - 0.01,
    firstRelease: true,
  });
  assert.equal(under.admitted, false);
  assert.ok(under.refusals.includes('DAILY_CAP'));
  assert.equal(under.daily_cap_usd, total - 0.01);
  const exact = await offlineAppPlan(market(), VARS, { dailyCapUsd: total, firstRelease: true });
  assert.ok(!exact.refusals.includes('DAILY_CAP'));
  for (const bad of [undefined, NaN, -1, 0, '5'])
    await assert.rejects(
      offlineAppPlan(market(), VARS, { dailyCapUsd: bad, firstRelease: true }),
      /daily cap/,
      String(bad),
    );
});
test('spend already booked today counts against the cap', async () => {
  const open = await offlineAppPlan(market(), VARS, { dailyCapUsd: 1000, firstRelease: true });
  const plan = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: open.estimate_usd + 1,
    spentTodayUsd: 2,
    firstRelease: true,
  });
  assert.ok(plan.refusals.includes('DAILY_CAP'));
});
test('the plan is deterministic and rents nothing', async () => {
  const a = await offlineAppPlan(market(), VARS, { dailyCapUsd: 1000, firstRelease: true });
  const b = await offlineAppPlan(market().reverse(), VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
  });
  assert.deepEqual(ids(a), ids(b));
  assert.deepEqual(
    a.legs.map((l) => l.offer_id),
    b.legs.map((l) => l.offer_id),
  );
});
test('nothing in src/ imports the planner: the Worker start kind app is not built', () => {
  for (const name of readdirSync(here('../src/')))
    if (name.endsWith('.js') && name !== 'app-lane.js')
      assert.ok(!/app-lane/.test(readFileSync(here('../src/' + name), 'utf8')), name);
});
test('command line: --changed-files and --app-lane (exit 2 when refused)', () => {
  const dir = mkdtempSync(join(tmpdir(), 'kf-app-lane-'));
  try {
    const run = (args) =>
      spawnSync('node', [new URL('../release-plan.mjs', import.meta.url).pathname, ...args], {
        encoding: 'utf8',
        env: { PATH: process.env.PATH },
      });
    writeFileSync(join(dir, 'a.txt'), 'docs/a.md\ncuda/walk/walk.cu\n');
    writeFileSync(join(dir, 'b.txt'), 'docs/a.md\nci/vast-broker/src/policy.js\n');
    const yes = run(['--changed-files', join(dir, 'a.txt')]);
    assert.equal(yes.status, 0, yes.stderr);
    assert.match(yes.stdout, /app lane REQUIRED\n {2}core: cuda\/walk\/walk\.cu/);
    assert.match(run(['--changed-files', join(dir, 'b.txt')]).stdout, /app lane not required/);
    writeFileSync(join(dir, 'm.json'), JSON.stringify({ offers: market() }));
    const base = [
      '--app-lane',
      '--market',
      join(dir, 'm.json'),
      '--first-release',
      '--var',
      'MAX_CONCURRENT_RENTALS=9',
    ];
    const ok = run([...base, '--daily-cap', '1000']);
    assert.equal(ok.status, 0, ok.stderr);
    assert.match(ok.stdout, /mode=first_release .* admitted=true/);
    const over = run([...base, '--daily-cap', '1']);
    assert.equal(over.status, 2, over.stderr);
    assert.match(over.stdout, /refusals=DAILY_CAP/);
    assert.notEqual(run([...base]).status, 0, 'no daily cap, no plan');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// ---- (8) images are bundles; an application list is resolved by bundlesFor (2026-11-19, docs/design/V3_PROPIMAGE_CONFIG.md) -------------------
// The generated plans did not change: the text of the two CLI plans below was captured from the tree before the planner read the bundle table.
import { BUNDLES } from '../src/bundles.js';

const cli = (...args) =>
  spawnSync(
    process.execPath,
    [
      fileURLToPath(here('../release-plan.mjs')),
      '--app-lane',
      '--market',
      fileURLToPath(here('./fixtures/release-market-synthetic.json')),
      '--daily-cap',
      '1000',
      ...args,
    ],
    { encoding: 'utf8' },
  );
const golden = (name) => readFileSync(here('./fixtures/app-lane/' + name), 'utf8');
const rotationTable = () => fileURLToPath(here('./fixtures/app-lane/rotation-table.json'));

test('the first-release plan is byte-identical to the one captured before the bundle table', () => {
  const r = cli('--first-release');
  assert.equal(r.status, 2);
  assert.equal(r.stdout, golden('plan-first.golden.txt'));
});
test('the rotation plan is byte-identical to the one captured before the bundle table', () => {
  const r = cli('--rotation', rotationTable());
  assert.equal(r.stdout, golden('plan-rotation.golden.txt'));
});
test('the golden plans are not trivial', () => {
  assert.match(golden('plan-first.golden.txt'), /imga on ada die=AD104/);
  assert.match(golden('plan-rotation.golden.txt'), /mode=rotation release_seq=4/);
});
test('every configured image is a bundle of the table, and a validation failure names it', () => {
  assert.deepEqual(validateAppLaneConfig(APP_LANE), []);
  const bad = structuredClone(APP_LANE);
  bad.images[0].id = 'imgz';
  assert.deepEqual(
    validateAppLaneConfig(bad).filter((e) => e.includes('not a bundle')),
    ['image imgz: not a bundle of propimage-bundles.json'],
  );
  const empty = structuredClone(BUNDLES);
  delete empty.bundles.imga;
  assert.ok(
    validateAppLaneConfig(APP_LANE, undefined, empty).includes(
      'image imga: not a bundle of propimage-bundles.json',
    ),
  );
});
test('an image id that is an inherited property name is not a bundle', () => {
  const bad = structuredClone(APP_LANE);
  bad.images[0].id = '__proto__';
  assert.ok(validateAppLaneConfig(bad).some((e) => e.includes('__proto__: not a bundle')));
});
test('an image id that is a property of Object is not a bundle', () => {
  const bad = structuredClone(APP_LANE);
  bad.images[0].id = 'constructor';
  assert.ok(validateAppLaneConfig(bad).some((e) => e.includes('constructor: not a bundle')));
});
test('--apps picks the images that hold the applications, and the plan is the full plan when both images are needed', () => {
  const r = cli('--first-release', '--apps', 'solidworks,premiere');
  assert.equal(r.stdout, golden('plan-first.golden.txt'));
});
test('--apps with one image plans that image only, like --images', () => {
  const byApps = cli('--first-release', '--apps', 'premiere');
  const byImages = cli('--first-release', '--images', 'imgb');
  assert.equal(byApps.stdout, byImages.stdout);
  assert.match(byApps.stdout, /imgb on ada/);
  assert.doesNotMatch(byApps.stdout, /imga on/);
});
test('--apps names the bundle that is cheaper, over rotation too', () => {
  const r = cli('--rotation', rotationTable(), '--apps', 'solidworks');
  assert.match(r.stdout, /imga on/);
  assert.doesNotMatch(r.stdout, /imgb on/);
});
test('an application on no bundle is refused by name', () => {
  const r = cli('--first-release', '--apps', 'solidworks,nope');
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /UNKNOWN_APP:nope/);
});
test('an application on a bundle that is not an app-lane image is refused by name', () => {
  const r = cli('--first-release', '--apps', 'matlab');
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /unknown image: imgc/);
});
test('apps and images are alternatives', () => {
  const r = cli('--first-release', '--apps', 'solidworks', '--images', 'imga');
  assert.notEqual(r.status, 0);
  assert.match(r.stderr, /apps and images are alternatives/);
});
test('the planner resolves apps through bundlesFor in the API too', async () => {
  const plan = await offlineAppPlan(market(), VARS, {
    dailyCapUsd: 1000,
    firstRelease: true,
    apps: ['premiere'],
  });
  assert.deepEqual(plan.images, ['imgb']);
  await assert.rejects(
    offlineAppPlan(market(), VARS, {
      dailyCapUsd: 1000,
      firstRelease: true,
      apps: ['premiere'],
      images: ['imgb'],
    }),
    /alternatives/,
  );
});
