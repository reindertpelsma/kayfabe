// The bandwidth estimate of a leg (src/bandwidth.js, bandwidth.json). GPU-free, no network, nothing rented.
// FALSIFIER (written before the code): a leg with no proprietary image costs as much as one with images; an image or stage that the
// leg does not run adds bytes; a size is copied instead of read from its pin; the margin is applied before the sum or not at all; a
// number lives in the .js; the breakdown names something that is not a line of the table; or the JavaScript and Python readings of
// the table disagree.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  BANDWIDTH,
  estimateBandwidth,
  plannedStages,
  bandwidthOverrides,
} from '../src/bandwidth.js';
import { buildInstruction, PROPIMAGES } from '../src/propimages.js';
import { resourceCaps } from '../src/policy.js';
import { RELEASE_MATRIX } from '../src/release-matrix.js';

const here = (p) => new URL(p, import.meta.url).pathname;
const REPO = here('../../../');
const GB = 1e9;
const python = (args, env = {}) =>
  spawnSync('python3', ['-B', '-I', ...args], {
    encoding: 'utf8',
    timeout: 60000,
    cwd: REPO,
    env: { PATH: process.env.PATH, ...env },
  });
const objects = Object.fromEntries(PROPIMAGES.objects.map((o) => [o.id, o]));

const CHECK_MIDDLE = ['check', { tierName: 'middle', linuxApps: true, winApps: 'mid' }];
const CHECK_SMALL = ['check', { tierName: 'small' }];
const slot = (tierName, die, releaseSeq) => [
  'release',
  {
    tierName,
    windowsRuns: tierName === 'large' ? 2 : 1,
    winApps: 'mid',
    linuxApps: true,
    propimages: true,
    die,
    releaseSeq,
  },
];
const A_ONLY = slot('large', 'AD102', 2);
const B_ONLY = slot('middle', 'AD104', 2);
const BOTH = [
  'release',
  {
    tierName: 'large',
    windowsRuns: 2,
    stages: ['specviewperf'],
    spec: 'lanea',
    winApps: 'mid',
    linuxApps: true,
    rehearsal: true,
    propimages: true,
    die: 'AD102',
  },
];
const make = ([profile, options]) => buildInstruction(profile, options);
const est = (leg, tier, options) => estimateBandwidth(make(leg), tier, options);
const sumOf = (e, key) => e.items.reduce((n, i) => n + i[key], 0);
const ids = (e) => e.items.map((i) => i.id);

test('a leg with few stages and no image is far smaller than a release leg with an image', () => {
  const small = est(CHECK_SMALL, 'small');
  const middle = est(CHECK_MIDDLE, 'middle');
  const release = est(A_ONLY, 'large');
  assert.ok(small.down_gb < middle.down_gb, 'small < middle');
  assert.ok(middle.down_gb * 2 < release.down_gb, 'middle is less than half of a release leg');
  assert.ok(small.down_gb * 3 < release.down_gb);
});
test('a leg with no image carries no propimage line and no stage the leg does not run', () => {
  const middle = est(CHECK_MIDDLE, 'middle');
  assert.equal(
    ids(middle).some((id) => id.startsWith('propimage_apps')),
    false,
  );
  assert.deepEqual(middle.stages, [
    'provision',
    'cuda_host',
    'merge_bar',
    'cuda_guest',
    'apps',
    'kfml',
    'windows_gate',
    'windows_apps',
  ]);
  assert.equal(ids(middle).includes('specviewperf.archive'), false);
});
test('image A only: exactly image A and its own extras', () => {
  const e = est(A_ONLY, 'large');
  const image = e.items.filter((i) => i.id === 'propimage_apps.image');
  assert.deepEqual(
    image.map((i) => [i.label, i.down]),
    [['imga', objects.imga.size]],
  );
  assert.ok(ids(e).includes('propimage_apps.imga_media'));
  assert.equal(ids(e).includes('propimage_apps.imgb_test_assets'), false);
});
test('image B only: exactly image B and its own extras', () => {
  const e = est(B_ONLY, 'middle');
  const image = e.items.filter((i) => i.id === 'propimage_apps.image');
  assert.deepEqual(
    image.map((i) => [i.label, i.down]),
    [['imgb', objects.imgb.size]],
  );
  assert.ok(ids(e).includes('propimage_apps.imgb_test_assets'));
  assert.equal(ids(e).includes('propimage_apps.imga_media'), false);
});
test('images A and B on one box (the one-die rehearsal): both, and the GOP once', () => {
  const e = est(BOTH, 'large');
  assert.deepEqual(
    e.items.filter((i) => i.id === 'propimage_apps.image').map((i) => [i.label, i.down]),
    [
      ['imga', objects.imga.size],
      ['imgb', objects.imgb.size],
    ],
  );
  assert.equal(ids(e).filter((id) => id === 'propimage_apps.gop').length, 1);
});
test('the split puts about half the image bytes on each box of a family', () => {
  const a = est(A_ONLY, 'large');
  const b = est(B_ONLY, 'middle');
  const both = est(BOTH, 'large');
  const imageBytes = (e) =>
    e.items.filter((i) => i.id === 'propimage_apps.image').reduce((n, i) => n + i.down, 0);
  assert.equal(imageBytes(a) + imageBytes(b), imageBytes(both));
  assert.ok(imageBytes(a) < 0.55 * imageBytes(both));
  assert.ok(imageBytes(b) < 0.6 * imageBytes(both));
  assert.ok(a.down_gb < both.down_gb && b.down_gb < both.down_gb);
});
test('an image size is the pinned size in propimages.json, never a copy: edit the pin and the estimate follows', () => {
  const edited = structuredClone(PROPIMAGES);
  edited.objects.find((o) => o.id === 'imga').size = 1000000000;
  const before = est(A_ONLY, 'large');
  const after = est(A_ONLY, 'large', { propimages: edited });
  assert.equal(
    before.items.find((i) => i.id === 'propimage_apps.image').down -
      after.items.find((i) => i.id === 'propimage_apps.image').down,
    objects.imga.size - 1000000000,
  );
});
test('a driver installer size is read from the release matrix', () => {
  const e = est(A_ONLY, 'large');
  const pin = RELEASE_MATRIX.driver_installers.find(
    (p) => p.os + '/' + p.version === 'linux/580.159.04',
  );
  assert.equal(e.items.find((i) => i.id === 'provision.host_driver').down, pin.size);
});
test('a stage skipped at the payload adds nothing', () => {
  const full = est(A_ONLY, 'large');
  const skipped = est(A_ONLY, 'large', { skip: ['windows_gate_2', 'apps'] });
  assert.equal(skipped.stages.includes('windows_gate_2'), false);
  assert.equal(
    skipped.items.some((i) => i.stage === 'apps' || i.stage === 'windows_gate_2'),
    false,
  );
  assert.ok(skipped.down_bytes < full.down_bytes);
});
test('--only names a stage, and only that stage and the fixed lines are counted', () => {
  const e = est(A_ONLY, 'large', { only: ['propimage_apps'] });
  assert.deepEqual(e.stages, ['propimage_apps']);
  assert.deepEqual([...new Set(e.items.map((i) => i.stage))].sort(), ['fixed', 'propimage_apps']);
});
test('a stage that is not in the leg plan is refused by name', () => {
  assert.throws(
    () => est(CHECK_MIDDLE, 'middle', { only: ['specviewperf'] }),
    /bandwidth stage:specviewperf/,
  );
  assert.throws(
    () => est(CHECK_MIDDLE, 'middle', { skip: ['propimage_apps'] }),
    /bandwidth stage:propimage_apps/,
  );
});
test('the manual-only stage with no image list counts the default image', () => {
  const doc = make([
    'release',
    { tierName: 'large', windowsRuns: 2, winApps: 'mid', linuxApps: true },
  ]);
  const e = estimateBandwidth(doc, 'large', { only: ['propimage_apps'] });
  assert.deepEqual(
    e.items.filter((i) => i.id === 'propimage_apps.image').map((i) => i.label),
    BANDWIDTH.default_images,
  );
});
test('the margin is applied last, to the sum, and is configurable', () => {
  const e = est(A_ONLY, 'large');
  assert.equal(e.margin_percent, BANDWIDTH.margin.percent);
  assert.equal(e.down_bytes, Math.ceil((sumOf(e, 'down') * (100 + e.margin_percent)) / 100));
  assert.equal(e.up_bytes, Math.ceil((sumOf(e, 'up') * (100 + e.margin_percent)) / 100));
  const none = est(A_ONLY, 'large', { marginPercent: 0 });
  assert.equal(none.down_bytes, sumOf(none, 'down'));
  const fifty = est(A_ONLY, 'large', { marginPercent: 50 });
  assert.equal(fifty.down_bytes, Math.ceil(sumOf(fifty, 'down') * 1.5));
});
test('an invalid margin is refused', () => {
  for (const marginPercent of [-1, NaN, 'x'])
    assert.throws(() => est(A_ONLY, 'large', { marginPercent }), /bandwidth margin/);
});
test('the GB figures are the bytes in decimal GB, rounded up to two places', () => {
  const e = est(A_ONLY, 'large');
  assert.equal(e.down_gb, Math.ceil((e.down_bytes / GB) * 100) / 100);
  assert.equal(e.up_gb, Math.ceil((e.up_bytes / GB) * 100) / 100);
});
test('a size the Worker read from an R2 list replaces the fallback of an r2-listed entry', () => {
  const plain = est(CHECK_MIDDLE, 'middle');
  const listed = est(CHECK_MIDDLE, 'middle', { r2Sizes: { 'winapps-open.iso': 123456789 } });
  const bundle = (e) => e.items.find((i) => i.id === 'windows_apps.bundle');
  assert.equal(bundle(plain).down, 7000000000);
  assert.equal(bundle(listed).down, 123456789);
  assert.equal(bundle(listed).from, 'r2 list');
});
test("an R2 size also replaces the fallback of image B's test assets", () => {
  const e = est(B_ONLY, 'middle', { r2Sizes: { 'test-assets-20261116b.iso': 42 } });
  assert.equal(e.items.find((i) => i.id === 'propimage_apps.imgb_test_assets').down, 42);
});
test('the uploads of the stages are summed, with the margin', () => {
  const e = est(A_ONLY, 'large');
  const uploads = e.stages.reduce((n, name) => n + BANDWIDTH.stages[name].upload.up_bytes, 0);
  assert.equal(sumOf(e, 'up'), uploads);
  assert.ok(e.up_bytes > uploads);
});
test('a repo size passed in moves the total by exactly the difference', () => {
  const a = est(CHECK_SMALL, 'small', { marginPercent: 0 });
  const b = est(CHECK_SMALL, 'small', { marginPercent: 0, repoBytes: 1000 });
  const repo = a.items.find((i) => i.id === 'fixed.repo_clone').down;
  assert.equal(a.down_bytes - b.down_bytes, repo - 1000);
});
test('a repo size passed in replaces the repo entry', () => {
  const e = est(CHECK_SMALL, 'small', { repoBytes: 55555 });
  assert.equal(e.items.find((i) => i.id === 'fixed.repo_clone').down, 55555);
  assert.notEqual(
    est(CHECK_SMALL, 'small').items.find((i) => i.id === 'fixed.repo_clone').down,
    55555,
  );
});
test('official URLs and private objects are counted the same way: bytes are bytes', () => {
  const e = est(A_ONLY, 'large');
  const classes = new Set(e.items.filter((i) => i.down > 0).map((i) => i.class));
  assert.ok(classes.has('r2-pinned') && classes.has('official-url') && classes.has('package'));
  assert.equal(e.down_bytes, Math.ceil((sumOf(e, 'down') * (100 + e.margin_percent)) / 100));
});
test('the SPEC lane adds the archive, and only when the instruction plans the stage', () => {
  assert.equal(ids(est(BOTH, 'large')).includes('specviewperf.archive'), true);
  assert.equal(ids(est(A_ONLY, 'large')).includes('specviewperf.archive'), false);
});
test('an entry with a `when` follows the instruction: image B assets only with image B', () => {
  const doc = make(A_ONLY);
  doc.propimages = []; // the same leg without the list: the stage is not planned and nothing of it is counted
  assert.equal(
    ids(estimateBandwidth(doc, 'large')).some((id) => id.startsWith('propimage_apps')),
    false,
  );
});
test('a table without a stage the leg runs is refused by name', () => {
  const broken = structuredClone(BANDWIDTH);
  delete broken.stages.windows_apps;
  assert.throws(
    () => est(CHECK_MIDDLE, 'middle', { table: broken }),
    /bandwidth table stage:windows_apps/,
  );
});
test('an unresolvable size_from is refused by name', () => {
  const broken = structuredClone(BANDWIDTH);
  broken.stages.provision.entries[0].size_from = 'driver:linux/1.0';
  assert.throws(
    () => est(CHECK_SMALL, 'small', { table: broken }),
    /bandwidth size_from:provision.host_driver/,
  );
});
test("the plan of stages is the payload's own plan (every leg of this file, with and without --only)", () => {
  const dir = mkdtempSync(join(tmpdir(), 'kf-bw-'));
  try {
    const legs = { CHECK_SMALL, CHECK_MIDDLE, A_ONLY, B_ONLY, BOTH };
    for (const [name, leg] of Object.entries(legs))
      for (const [only, skip] of [[[], []], [['install_flow'], []], [[], ['kfml']]]) {
        const doc = make(leg);
        if (only.includes('install_flow') && doc.tier !== 'large') continue;
        if (skip.includes('kfml') && !['middle', 'large'].includes(doc.tier)) continue; // the payload refuses a --skip of a stage that is not planned
        const file = join(dir, name + '.json');
        writeFileSync(file, JSON.stringify(doc));
        const args = ['scripts/ci/vast_payload.py', '--instruction', file, '--dry-run'];
        for (const o of only) args.push('--only', o);
        for (const o of skip) args.push('--skip', o);
        const run = python(args);
        // `planned=` is the whole plan; `stages=` is what --only/--skip chose out of it (a --skip leg is compared with the chosen list)
        const planned = (skip.length ? /stages=(\S+) planned=/ : /planned=(\S+)/).exec(run.stdout)?.[1];
        // image B's definitions are another change: a strict refusal there is not a plan to compare
        if (!planned) {
          assert.match(run.stdout, /STRICT_UNKNOWN_APP/, name);
          continue;
        }
        assert.deepEqual(plannedStages(doc, { only, skip }), planned.split(':'), name + only + skip);
      }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
test('the Python reading of the table selects the same lines and bytes as the JavaScript one', () => {
  const dir = mkdtempSync(join(tmpdir(), 'kf-bw-'));
  try {
    const legs = { CHECK_SMALL, CHECK_MIDDLE, A_ONLY, B_ONLY, BOTH };
    for (const [name, leg] of Object.entries(legs)) {
      const doc = make(leg);
      const file = join(dir, name + '.json');
      writeFileSync(file, JSON.stringify(doc));
      const run = python(['scripts/ci/bandwidth_check.py', '--explain', file, '--tier', doc.tier]);
      assert.equal(run.status, 0, run.stderr);
      const js = estimateBandwidth(doc, doc.tier).items.map((i) => ({
        id: i.id,
        stage: i.stage,
        label: i.label ?? null,
        down: i.down,
        up: i.up,
      }));
      assert.deepEqual(JSON.parse(run.stdout), js, name);
    }
    // the manual-only stage with no image list, and a skipped stage
    const doc = make([
      'release',
      { tierName: 'large', windowsRuns: 2, winApps: 'mid', linuxApps: true },
    ]);
    const file = join(dir, 'manual.json');
    writeFileSync(file, JSON.stringify(doc));
    for (const [flag, value] of [
      ['--only', 'propimage_apps'],
      ['--skip', 'windows_gate_2'],
    ]) {
      const run = python(['scripts/ci/bandwidth_check.py', '--explain', file, flag, value]);
      assert.equal(run.status, 0, run.stderr);
      const options = flag === '--only' ? { only: [value] } : { skip: [value] };
      const js = estimateBandwidth(doc, doc.tier, options).items.map((i) => ({
        id: i.id,
        stage: i.stage,
        label: i.label ?? null,
        down: i.down,
        up: i.up,
      }));
      assert.deepEqual(JSON.parse(run.stdout), js, flag);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
test('the table passes the guard: every stage of the payload, provenance on every number', () => {
  const run = python(['scripts/ci/bandwidth_check.py', '--today', '2026-11-17']);
  assert.equal(run.status, 0, run.stdout + run.stderr);
  assert.match(run.stdout, /bandwidth table: PASS/);
});
// ---- the breakdown the Worker logs
test('every line of the breakdown names an entry of the table (the one line to edit)', () => {
  const e = est(BOTH, 'large');
  const table = new Set([
    ...BANDWIDTH.fixed.map((x) => x.id),
    ...Object.entries(BANDWIDTH.stages).flatMap(([name, s]) => [
      ...s.entries.map((x) => x.id),
      name + '.upload',
    ]),
  ]);
  for (const item of e.items) {
    assert.ok(table.has(item.id), item.id);
    assert.ok(
      e.lines.some((l) => l.includes(item.id + ' ') || l.includes(item.id + ':')),
      item.id,
    );
  }
});
test('the breakdown says what the number is made of, in GB, with the margin and the totals', () => {
  const lines = est(A_ONLY, 'large').lines;
  assert.match(
    lines[0],
    /^bandwidth estimate \(ci\/vast-broker\/bandwidth\.json\) tier=large stages=provision:.*images=imga$/,
  );
  assert.ok(
    lines.some((l) =>
      /50\.497 GB down \[measured r2-pinned\] propimage_apps\.image \(imga\)/.test(l),
    ),
  );
  assert.ok(lines.some((l) => /^\+15 percent margin: [0-9.]+ GB down, [0-9.]+ GB up$/.test(l)));
});
test('the breakdown marks every ESTIMATE so the owner sees which numbers are guesses', () => {
  const lines = est(A_ONLY, 'large').lines.filter((l) => l.includes('provision.packages'));
  assert.match(lines[0], /\[ESTIMATE package\]/);
});
// ---- no number in the code
test('the module holds no data number: only unit conversions and formatting digits', () => {
  const code = readFileSync(here('../src/bandwidth.js'), 'utf8')
    .split('\n')
    .filter((l) => !l.trim().startsWith('//'))
    .join('\n')
    .replace(/'[^']*'/g, "''");
  const numbers = [...code.matchAll(/(?<![A-Za-z_.$])\d+(?:\.\d+)?(?:e\d+)?/g)].map((m) => m[0]);
  const allowed = new Set(['0', '1', '1e9', '100', '2', '3', '8']);
  assert.deepEqual(
    numbers.filter((n) => !allowed.has(n)),
    [],
  );
});
// ---- pricing
test('the overrides are the estimate rounded up to whole GB', () => {
  const e = est(A_ONLY, 'large');
  assert.deepEqual(bandwidthOverrides(e), {
    bandwidthDownOverride: Math.ceil(e.down_gb),
    bandwidthUpOverride: Math.ceil(e.up_gb),
  });
});
test('policy.js takes max(profile row, override): an estimate below the profile row does not lower the price yet, one above raises it', () => {
  const profile = resourceCaps({}, 'release').bandwidthDownGb;
  const low = resourceCaps(
    bandwidthOverrides({ down_gb: 1, up_gb: 0.01 }),
    'release',
  ).bandwidthDownGb;
  const high = resourceCaps(
    bandwidthOverrides({ down_gb: profile + 50, up_gb: 3 }),
    'release',
  ).bandwidthDownGb;
  assert.equal(low, profile);
  assert.equal(high, profile + 50);
});
