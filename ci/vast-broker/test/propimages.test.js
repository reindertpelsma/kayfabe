// The Worker's proprietary-image configuration and the instruction built from it (src/propimages.js, propimages.json,
// docs/design/V3_WORKER_PROPIMAGES.md). GPU-free, no network, nothing rented.
// FALSIFIER (written before the code): the instruction the Worker would build differs from what the box and the standalone
// command line read for the same leg (the golden files, the box's own parser, `--emit-instruction`, the runner's flag
// parser); a config that is not strict, names an unpinned object, an unknown image or an unknown application is accepted;
// the part is emitted without being asked for; or the private objects reach the /bench-data allow-list.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  PROPIMAGES,
  MAX_IMAGES,
  propimagesConfigProblems,
  checkPropimagesConfig,
  propimageLabelsFor,
  propimagesForLeg,
  propimagePlan,
  slotOfDie,
  buildLeg,
  buildInstruction,
  instructionFromEnvironment,
  canonicalInstruction,
} from '../src/propimages.js';
import { tierEnvironment } from '../src/tier.js';
import { BENCH_OBJECTS } from '../src/benchdata.js';
import { RELEASE_MATRIX } from '../src/release-matrix.js';

const here = (p) => new URL(p, import.meta.url).pathname;
const REPO = here('../../../');
const read = (p) => readFileSync(p, 'utf8');
const golden = (name) => read(here('./fixtures/propimages/' + name + '.instruction.json'));
const python = (script, args, env = {}) =>
  spawnSync('python3', ['-B', '-I', script, ...args], {
    encoding: 'utf8',
    timeout: 60000,
    cwd: REPO,
    env: { PATH: process.env.PATH, ...env },
  });
const check = (input) => {
  const run = spawnSync('python3', ['-B', '-I', here('./propimages_check.py')], {
    input: JSON.stringify(input),
    encoding: 'utf8',
    timeout: 60000,
  });
  assert.equal(run.status, 0, run.stderr);
  return JSON.parse(run.stdout);
};
const edit = (fn) => {
  const c = structuredClone(PROPIMAGES);
  fn(c);
  return c;
};
const image = (c, label) => c.images.find((i) => i.label === label);
const refuses = (c, token, options) => {
  const problems = propimagesConfigProblems(c, options);
  assert.ok(problems.includes(token), token + ' not in ' + JSON.stringify(problems));
};
// The rehearsal leg is the runnable release scope (docs/design/V3_RELEASE_REHEARSAL.md): large, one die.
const REHEARSAL = {
  tierName: 'large',
  windowsRuns: 2,
  stages: ['specviewperf'],
  spec: 'lanea',
  winApps: 'mid',
  linuxApps: true,
  rehearsal: true,
};
// A release leg of the two-die split (profile release, no driver matrix: the box refuses a matrix leg today). `releaseSeq` is
// the release number the request carries; even keeps the configured slots, odd exchanges them.
const SLOT_LEG = (tierName, die, releaseSeq) => ({
  tierName,
  windowsRuns: tierName === 'large' ? 2 : 1,
  winApps: 'mid',
  linuxApps: true,
  propimages: true,
  die,
  releaseSeq,
});
const LEGS = {
  'ad102-large-rehearsal': { ...REHEARSAL, propimages: true, die: 'AD102' },
  'ad102-large-slot1-even': SLOT_LEG('large', 'AD102', 2),
  'ad102-large-slot1-odd': SLOT_LEG('large', 'AD102', 3),
  'ad104-middle-slot2-even': SLOT_LEG('middle', 'AD104', 2),
};
const names = Object.keys(LEGS);
const build = (options, config) => buildInstruction('release', options, config);
const registry = () => check({ mode: 'registry' }).ids;

// ---------------------------------------------------------------- the configuration
test('the committed configuration is valid and strict', () => {
  assert.deepEqual(propimagesConfigProblems(), []);
  assert.equal(PROPIMAGES.strict, true);
  assert.doesNotThrow(() => checkPropimagesConfig());
});
test('every pinned object equals the pin the box reads (size, sha256, ref)', () => {
  const pins = JSON.parse(read(REPO + 'scripts/ci/propimage_apps_pins.json')).items;
  // Image B's pin file belongs to its own change (#446: scripts/ci/propimage_apps_b_pins.json). Until it is in this tree the
  // values below are the owner's: the zip is 62,506,351,001 bytes and its sha256 is the one measured by a full readback.
  const bPath = REPO + 'scripts/ci/propimage_apps_b_pins.json';
  const b = existsSync(bPath) ? JSON.parse(read(bPath)).items : [];
  const known = [...pins, ...b];
  for (const o of PROPIMAGES.objects) {
    const pin = known.find((p) => p.ref === o.ref);
    if (pin === undefined) {
      assert.equal(o.id, 'imgb', 'only image B may lack a pin file in this tree');
      assert.equal(o.size, 62506351001);
      assert.equal(o.sha256, 'fe75afa169e65770f974fbcc3a2cb018ff14d58bd39b298539128cc6edcc154a');
      continue;
    }
    assert.equal(pin.sha256, o.sha256, o.id + ' sha256');
    assert.equal(pin.size, o.size, o.id + ' size');
    assert.equal(pin.role, o.role, o.id + ' role');
  }
});
test('image A pin, image B pin and the signed GOP are the values the owner gave', () => {
  const by = Object.fromEntries(PROPIMAGES.objects.map((o) => [o.id, o]));
  assert.equal(by.imga.object, 'windows-proprietary/kfimg-20261112/image-v2.zip');
  assert.equal(by.imga.size, 50496618964);
  assert.equal(by.imga.sha256, 'cddff40a501d8135c6c15ef7267b3b838304e5d6f089502707bce78712cb11b3');
  assert.equal(by.imgb.object, 'windows-proprietary/kfimg-20261116/image.zip');
  assert.equal(by.imgb.size, 62506351001);
  assert.equal(by.imgb.sha256, 'fe75afa169e65770f974fbcc3a2cb018ff14d58bd39b298539128cc6edcc154a');
  assert.equal(by.gop.size, 11224);
  assert.equal(by.gop.sha256, '423a9abc5789b8b4e9834ba016261efb9d64515d53f259a90446917ce91b848e');
});
test('the lane table is the measured one: what passes today is gate, the rest is discovery', () => {
  const lanes = (label) => {
    const out = { gate: [], discovery: [] };
    for (const [id, row] of Object.entries(image(PROPIMAGES, label).apps)) out[row.lane].push(id);
    return { gate: out.gate.sort(), discovery: out.discovery.sort() };
  };
  assert.deepEqual(lanes('imga'), {
    gate: ['edge', 'edrawings', 'solidworks'],
    discovery: ['3dmark', 'resolve', 'superposition'],
  });
  assert.deepEqual(lanes('imgb'), {
    gate: ['acrobat', 'dimension', 'gpudiag', 'illustrator', 'lightroomlaunch', 'premiere', 'uxp'],
    discovery: [
      'aeparallel',
      'aeprojects',
      'aetoggle',
      'aftereffects',
      'indesign',
      'indesignsample',
      'illustratorsample',
      'mediaencoder',
      'photoshop',
      'photoshopprojects',
      'photoshopsample',
    ].sort(),
  });
});
test('an application with no driver or no definition is excluded, never listed (a strict run would fail on it)', () => {
  for (const img of PROPIMAGES.images)
    for (const id of Object.keys(img.excluded ?? {})) assert.equal(id in img.apps, false, id);
  assert.ok('wukong' in image(PROPIMAGES, 'imga').excluded);
  assert.equal('wukong' in image(PROPIMAGES, 'imga').apps, false);
});
test('image A applications all have a definition in the runner registry', () => {
  const ids = registry();
  const unknown = Object.keys(image(PROPIMAGES, 'imga').apps).filter((id) => !ids.includes(id));
  assert.deepEqual(unknown, []);
});

// refusals of the configuration, by name
test('a config that is not strict is refused (strict false)', () => {
  refuses(
    edit((c) => (c.strict = false)),
    'NOT_STRICT',
  );
});
test('a config that is not strict is refused (strict absent)', () => {
  refuses(
    edit((c) => delete c.strict),
    'NOT_STRICT',
  );
});
test('a non-strict config builds no instruction', () => {
  const c = edit((x) => (x.strict = false));
  assert.throws(
    () =>
      propimagesForLeg(
        { profile: 'release', tierName: 'large', die: 'AD102', releaseSeq: 2 },
        { config: c },
      ),
    /NOT_STRICT/,
  );
});
test('an unpinned object is refused: no sha256', () => {
  refuses(
    edit((c) => delete c.objects[0].sha256),
    'UNPINNED:imga',
  );
});
test('an unpinned object is refused: sha256 not 64 lower-case hex', () => {
  refuses(
    edit((c) => (c.objects[1].sha256 = 'FE75AFA1'.repeat(8))),
    'UNPINNED:imgb',
  );
  refuses(
    edit((c) => (c.objects[1].sha256 = 'fe75afa1')),
    'UNPINNED:imgb',
  );
});
test('an unpinned object is refused: no size or a size that is not a positive integer', () => {
  refuses(
    edit((c) => delete c.objects[0].size),
    'SIZE:imga',
  );
  refuses(
    edit((c) => (c.objects[0].size = 0)),
    'SIZE:imga',
  );
});
test('an unpinned object is refused: a size given as text', () => {
  refuses(
    edit((c) => (c.objects[1].size = '62506351001')),
    'SIZE:imgb',
  );
});
test('an image naming an object that does not exist is refused by name', () => {
  refuses(
    edit((c) => (image(c, 'imgb').object = 'imgz')),
    'UNKNOWN_OBJECT:imgb',
  );
});
test('an image naming the GOP object as its disk is refused by name', () => {
  refuses(
    edit((c) => (image(c, 'imga').object = 'gop')),
    'UNKNOWN_OBJECT:imga',
  );
});
test('a duplicate label is refused', () => {
  refuses(
    edit((c) => (c.images[1].label = 'imga')),
    'DUPLICATE_LABEL:imga',
  );
});
test('a config with a duplicate label builds no instruction', () => {
  const c = edit((x) => (x.images[1].label = 'imga'));
  assert.throws(
    () =>
      propimagesForLeg(
        { profile: 'release', tierName: 'large', die: 'AD102', releaseSeq: 2 },
        { config: c },
      ),
    /DUPLICATE_LABEL:imga/,
  );
});
test('a config with a duplicate object id builds no instruction', () => {
  const c = edit((x) => (x.objects[1].id = 'imga'));
  assert.throws(
    () =>
      propimagesForLeg(
        { profile: 'release', tierName: 'large', die: 'AD102', releaseSeq: 2 },
        { config: c },
      ),
    /DUPLICATE_OBJECT:imga/,
  );
});
test('a duplicate object id is refused', () => {
  refuses(
    edit((c) => (c.objects[1].id = 'imga')),
    'DUPLICATE_OBJECT:imga',
  );
});
test('a duplicate object ref is refused', () => {
  refuses(
    edit((c) => (c.objects[1].ref = c.objects[0].ref)),
    'DUPLICATE_REF',
  );
});
test('an image with no applications is refused', () => {
  refuses(
    edit((c) => (image(c, 'imga').apps = {})),
    'NO_APPS:imga',
  );
});
test('a lane that is neither gate nor discovery is refused by name', () => {
  refuses(
    edit((c) => (image(c, 'imga').apps.edge.lane = 'optional')),
    'LANE:imga:edge',
  );
});
test('an application with no lane is refused by name', () => {
  refuses(
    edit((c) => delete image(c, 'imgb').apps.uxp.lane),
    'LANE:imgb:uxp',
  );
});
test('a malformed application id is refused by name', () => {
  refuses(
    edit((c) => (image(c, 'imga').apps.Edge = { lane: 'gate' })),
    'APP_ID:imga:Edge',
  );
  refuses(
    edit((c) => (image(c, 'imga').apps['123'] = { lane: 'gate' })),
    'APP_ID:imga:123',
  );
});
test('an application the runner does not know is refused by name (image A)', () => {
  const c = edit((x) => (image(x, 'imga').apps.zzunknown = { lane: 'gate' }));
  const ids = registry();
  refuses(c, 'UNKNOWN_APP:imga:zzunknown', { registry: { imga: ids, imgb: ids } });
});
test('an application the runner does not know is refused by name (image B)', () => {
  const c = edit((x) => (image(x, 'imgb').apps.zzunknownb = { lane: 'discovery' }));
  const ids = registry();
  refuses(c, 'UNKNOWN_APP:imgb:zzunknownb', { registry: { imga: ids, imgb: ids } });
});
test('a measured family the platform does not have is refused by name', () => {
  refuses(
    edit((c) => (image(c, 'imgb').measured_families = ['hopper'])),
    'MEASURED_FAMILIES:imgb',
  );
});
test('a measured family listed twice is refused by name', () => {
  refuses(
    edit((c) => (image(c, 'imga').measured_families = ['ada', 'ada'])),
    'MEASURED_FAMILIES:imga',
  );
});
test('two images in one slot are refused', () => {
  refuses(
    edit((c) => (image(c, 'imgb').slot = 1)),
    'SLOT:imgb',
  );
});
test('a config with two images in one slot builds no instruction', () => {
  const c = edit((x) => (image(x, 'imgb').slot = 1));
  assert.throws(
    () =>
      propimagePlan(
        { profile: 'release', tierName: 'large', die: 'AD102', releaseSeq: 2 },
        { config: c },
      ),
    /SLOT:imgb/,
  );
});
test('a slot other than 1 or 2 is refused', () => {
  refuses(
    edit((c) => (image(c, 'imga').slot = 3)),
    'SLOT:imga',
  );
});
test('the release-level swap must be a boolean', () => {
  refuses(
    edit((c) => (c.swap = 'yes')),
    'SWAP',
  );
});
test('the release-level swap must be present', () => {
  refuses(
    edit((c) => delete c.swap),
    'SWAP',
  );
});
test('more than eight images are refused', () => {
  const c = edit((x) => {
    for (let n = 0; n < MAX_IMAGES; n++)
      x.images.push({ ...structuredClone(x.images[0]), label: 'x' + n });
  });
  refuses(c, 'IMAGES');
});
test('an application cannot be both listed and excluded', () => {
  refuses(
    edit((c) => (image(c, 'imga').excluded.edge = 'x')),
    'EXCLUDED:imga',
  );
});
test('an application the harness cannot grade stays refused when someone lists it (wukong on image A)', () => {
  refuses(
    edit((c) => (image(c, 'imga').apps.wukong = { lane: 'discovery' })),
    'EXCLUDED:imga',
  );
});
test('exactly one signed GOP object is required', () => {
  refuses(
    edit((c) => c.objects.pop()),
    'GOP',
  );
});

// ---------------------------------------------------------------- one leg: slot, swap, rotation, fallback
const plan = (leg, config) =>
  propimagePlan({ profile: 'release', ...leg }, config === undefined ? undefined : { config });
const labelsOf = (leg, config) => plan(leg, config).assignment.images;
const SLOT1 = { tierName: 'large', die: 'AD102', releaseSeq: 2 };
const SLOT2 = { tierName: 'middle', die: 'AD104', releaseSeq: 2 };
test('slot of a die is its place among its family rows in the release matrix order', () => {
  assert.equal(slotOfDie('AD102'), 1);
  assert.equal(slotOfDie('AD104'), 2);
  assert.equal(slotOfDie('TU106'), 1);
  assert.equal(slotOfDie('GB205'), 2);
});
test('a die that is not a release matrix row has no slot', () => {
  assert.throws(() => slotOfDie('GA106'), /propimages slot:GA106/);
  assert.throws(() => slotOfDie('XX999'), /propimages slot/);
});
test('slot 1 runs image A and only image A', () => {
  assert.deepEqual(labelsOf(SLOT1), ['imga']);
});
test('slot 2 runs image B and only image B', () => {
  assert.deepEqual(labelsOf(SLOT2), ['imgb']);
});
test('the two boxes of a family never run the same image and together run both', () => {
  for (const [a, b] of [
    ['AD102', 'AD104'],
    ['GA102', 'GA104'],
    ['TU106', 'TU116'],
    ['GB202', 'GB205'],
  ])
    for (const releaseSeq of [0, 1, 6, 7]) {
      const one = labelsOf({ tierName: 'large', die: a, releaseSeq });
      const two = labelsOf({ tierName: 'middle', die: b, releaseSeq });
      assert.equal(one.length + two.length, 2, a);
      assert.deepEqual([...one, ...two].sort(), ['imga', 'imgb'], a + releaseSeq);
    }
});
test('an odd release exchanges the images (slot 1 runs B, slot 2 runs A)', () => {
  assert.deepEqual(labelsOf({ ...SLOT1, releaseSeq: 3 }), ['imgb']);
  assert.deepEqual(labelsOf({ ...SLOT2, releaseSeq: 3 }), ['imga']);
});
test('an even release keeps them (including release 0)', () => {
  assert.deepEqual(labelsOf({ ...SLOT1, releaseSeq: 0 }), ['imga']);
  assert.deepEqual(labelsOf({ ...SLOT2, releaseSeq: 0 }), ['imgb']);
});
test('config swap on exchanges the images of an even release', () => {
  const c = edit((x) => (x.swap = true));
  assert.deepEqual(labelsOf(SLOT1, c), ['imgb']);
  assert.deepEqual(labelsOf(SLOT2, c), ['imga']);
});
test('config swap on and an odd release cancel out (XOR)', () => {
  const c = edit((x) => (x.swap = true));
  assert.deepEqual(labelsOf({ ...SLOT1, releaseSeq: 3 }, c), ['imga']);
  assert.deepEqual(labelsOf({ ...SLOT2, releaseSeq: 3 }, c), ['imgb']);
});
test('the same release number gives the same assignment, however often it is asked (a retry never moves an image)', () => {
  const first = JSON.stringify(plan({ ...SLOT1, releaseSeq: 41 }));
  for (let n = 0; n < 5; n++)
    assert.equal(JSON.stringify(plan({ ...SLOT1, releaseSeq: 41 })), first);
  // release 43 has the same parity: the same images, only the number recorded differs
  assert.equal(JSON.stringify(plan({ ...SLOT1, releaseSeq: 43 })), first.replace(/\b41\b/g, '43'));
});
test('consecutive releases alternate', () => {
  const run = (n) => labelsOf({ ...SLOT1, releaseSeq: n })[0];
  assert.deepEqual([0, 1, 2, 3].map(run), ['imga', 'imgb', 'imga', 'imgb']);
});
test('the release number is required for the two-die split and must be a non-negative integer', () => {
  for (const releaseSeq of [undefined, -1, 1.5, '2', null])
    assert.throws(
      () => plan({ ...SLOT1, releaseSeq }),
      /propimages releaseSeq/,
      String(releaseSeq),
    );
});
test('a single available die runs BOTH images and is not rotated', () => {
  for (const releaseSeq of [0, 1, undefined])
    assert.deepEqual(labelsOf({ tierName: 'large', die: 'AD102', familyDies: 1, releaseSeq }), [
      'imga',
      'imgb',
    ]);
});
test('a single-die fallback is the same for the second die of the family', () => {
  assert.deepEqual(labelsOf({ tierName: 'middle', die: 'AD104', familyDies: 1 }), ['imga', 'imgb']);
});
test('the one-die rehearsal runs both images without a release number', () => {
  const { instruction } = buildLeg('release', LEGS['ad102-large-rehearsal']);
  assert.deepEqual(
    instruction.propimages.map((i) => i.label),
    ['imga', 'imgb'],
  );
});
test('the release number is required through the whole instruction too', () => {
  const { releaseSeq, ...rest } = LEGS['ad102-large-slot1-even'];
  assert.throws(() => build(rest), /propimages releaseSeq/);
});
test('a small leg inside a release is refused through the whole instruction too', () => {
  assert.throws(
    () => build({ tierName: 'small', propimages: true, die: 'AD102', releaseSeq: 2 }),
    /propimages tier/,
  );
});
test('the family die count must be 1 or 2', () => {
  assert.throws(() => plan({ ...SLOT1, familyDies: 3 }), /propimages dies/);
  assert.throws(() => plan({ ...SLOT1, familyDies: 0 }), /propimages dies/);
});
test('the plan says which image runs here and why (the log text)', () => {
  const text = plan({ ...SLOT1, releaseSeq: 3 }).lines.join('\n');
  assert.match(
    text,
    /die AD102 \(ada\) is box slot 1 of 2; release 3 is odd and config swap is off, so the effective swap is on/,
  );
  assert.match(text, /image imgb: configured slot 2, box slot 1 -> RUNS HERE/);
  assert.match(text, /image imga: configured slot 1, box slot 2 \(runs on the other box\)/);
});
test('the single-die plan says that both images run on the one box', () => {
  const text = plan({ tierName: 'large', die: 'AD102', familyDies: 1 }).lines.join('\n');
  assert.match(
    text,
    /only one die of family ada is available: this box runs BOTH images \(imga, imgb\), no rotation/,
  );
});
test('the assignment record names slot, swap, rotation and release number', () => {
  const a = plan({ ...SLOT2, releaseSeq: 5 }).assignment;
  assert.deepEqual(
    {
      mode: a.mode,
      slot: a.slot,
      swap: a.swap,
      rotation: a.rotation,
      releaseSeq: a.releaseSeq,
      config_swap: a.config_swap,
    },
    { mode: 'split', slot: 2, swap: true, rotation: true, releaseSeq: 5, config_swap: false },
  );
});
// ---- lanes on a family the image has not run on
test('on a family the image has run on, the lanes are the configured ones', () => {
  const apps = plan(SLOT1).part.propimages[0].apps;
  assert.equal(apps.edge.lane, 'gate');
  assert.equal(apps.resolve.lane, 'discovery');
});
test('on a family the image has not run on, EVERY application is in the discovery lane', () => {
  // image B has only run on Ada: its box on Turing, Ampere and Blackwell gets no gate row
  for (const die of ['TU116', 'GA104', 'GB205']) {
    const apps = plan({ tierName: 'middle', die, releaseSeq: 2 }).part.propimages[0].apps;
    assert.ok(Object.keys(apps).length > 10, die);
    assert.deepEqual([...new Set(Object.values(apps).map((a) => a.lane))], ['discovery'], die);
  }
});
test('image A on blackwell (never run there) is all discovery, on ampere it keeps its gate rows', () => {
  const lanes = (die) =>
    new Set(
      Object.values(plan({ tierName: 'large', die, releaseSeq: 2 }).part.propimages[0].apps).map(
        (a) => a.lane,
      ),
    );
  assert.deepEqual([...lanes('GB202')], ['discovery']);
  assert.ok(lanes('GA102').has('gate'));
});
test('the plan names the unmeasured family and why the lanes changed', () => {
  const text = plan({ tierName: 'middle', die: 'TU116', releaseSeq: 2 }).lines.join('\n');
  assert.match(
    text,
    /imgb on turing: not yet measured on this family: every application in the discovery lane/,
  );
});
test('promoting a family is one config edit: add it to measured_families', () => {
  const c = edit((x) => image(x, 'imgb').measured_families.push('blackwell'));
  const apps = plan({ tierName: 'middle', die: 'GB205', releaseSeq: 2 }, c).part.propimages[0].apps;
  assert.equal(apps.gpudiag.lane, 'gate');
});
// ---- explicit selection, refusals
test('an unknown image label is refused by name', () => {
  assert.throws(() => plan({ ...SLOT1, images: ['imgz'] }), /propimages unknown image:imgz/);
});
test('a duplicate image selection is refused', () => {
  assert.throws(() => plan({ ...SLOT1, images: ['imga', 'imga'] }), /propimages images/);
});
test('a duplicate image selection is refused through the whole instruction too', () => {
  assert.throws(
    () => build({ ...LEGS['ad102-large-slot1-even'], propimageImages: ['imgb', 'imgb'] }),
    /propimages images/,
  );
});
test('an empty image selection is refused', () => {
  assert.throws(() => plan({ ...SLOT1, images: [] }), /propimages images/);
});
test('an unknown image label is refused by name through the whole instruction too', () => {
  assert.throws(
    () => build({ ...LEGS['ad102-large-slot1-even'], propimageImages: ['imga', 'imgz'] }),
    /propimages unknown image:imgz/,
  );
});
test('an explicit selection names its images and says the slot rule is not applied', () => {
  const p = plan({ ...SLOT1, images: ['imgb'] });
  assert.deepEqual(p.assignment.images, ['imgb']);
  assert.match(p.lines[0], /the slot rule is not applied/);
});
test('inside a release only the large and the middle leg take images', () => {
  assert.throws(() => plan({ ...SLOT1, tierName: 'small' }), /propimages tier/);
  assert.throws(() => plan({ ...SLOT1, tierName: 'adhoc' }), /propimages tier/);
  assert.throws(() => plan({ ...SLOT1, tierName: undefined }), /propimages tier/);
});
test('only the release profile takes images: small, middle and sweep never do, even with the flag', () => {
  for (const [profile, tierName] of [
    ['check', 'small'],
    ['check', 'middle'],
    ['check', 'large'],
    ['sweep', 'small'],
    ['sweep', 'adhoc'],
  ])
    assert.throws(
      () => buildInstruction(profile, { tierName, propimages: true, die: 'AD102', releaseSeq: 2 }),
      /propimages release only/,
      profile + '/' + tierName,
    );
});
test('a die of a family outside the release dies is refused by name', () => {
  assert.throws(
    () =>
      propimagePlan(
        { ...SLOT1, profile: 'release' },
        { cfg: { releaseDies: { turing: ['TU106'] } } },
      ),
    /propimages family:ada/,
  );
});
test('an unknown die is refused by name', () => {
  assert.throws(() => plan({ ...SLOT1, die: 'XX999' }), /propimages die/);
  assert.throws(() => plan({ ...SLOT1, die: undefined }), /propimages die/);
});
test('the labels a leg takes are [] for a leg that cannot take any', () => {
  assert.deepEqual(propimageLabelsFor({ profile: 'check', tierName: 'small', die: 'AD102' }), []);
  assert.deepEqual(
    propimageLabelsFor({ profile: 'release', tierName: 'large', die: 'AD102', releaseSeq: 2 }),
    ['imga'],
  );
});
test('the part is always strict and carries the lane of every application', () => {
  const part = plan(SLOT1).part;
  assert.equal(part.propimages_strict, true);
  for (const img of part.propimages) {
    assert.equal(img.kind, 'prebuilt');
    assert.equal(img.disk.from, 'pin');
    for (const [id, row] of Object.entries(img.apps)) {
      assert.equal(row.mode, 'present', id);
      assert.equal(row.lane, image(PROPIMAGES, img.label).apps[id].lane, id);
    }
  }
});
test('the disk carries the sha256 pin of the object, so the box checks it twice', () => {
  const part = plan({ tierName: 'large', die: 'AD102', familyDies: 1 }).part;
  const objects = Object.fromEntries(PROPIMAGES.objects.map((o) => [o.id, o]));
  assert.deepEqual(
    part.propimages.map((i) => [i.label, i.disk.ref, i.disk.sha256]),
    [
      ['imga', objects.imga.ref, objects.imga.sha256],
      ['imgb', objects.imgb.ref, objects.imgb.sha256],
    ],
  );
});
test('the instruction never carries an R2 object key, a size or the reasons of the table', () => {
  const text = canonicalInstruction(build(LEGS['ad102-large-rehearsal']));
  for (const o of PROPIMAGES.objects) assert.equal(text.includes(o.object), false, o.object);
  assert.equal(text.includes('windows-proprietary'), false);
  assert.equal(text.includes('basis'), false);
  assert.equal(text.includes('50496618964'), false);
});

// ---------------------------------------------------------------- default off, nothing live changes
test('without the flag there is no propimages key at all', () => {
  const doc = buildInstruction('release', { ...REHEARSAL, die: 'AD102' });
  assert.equal('propimages' in doc, false);
  assert.equal('propimages_strict' in doc, false);
  assert.equal('propimages' in buildInstruction('check', { tierName: 'small' }), false);
});
test('the flag must be exactly true (text)', () => {
  assert.throws(
    () => buildInstruction('release', { ...REHEARSAL, propimages: 'yes', die: 'AD102' }),
    /propimages flag/,
  );
});
test('the flag must be exactly true (number)', () => {
  assert.throws(
    () => buildInstruction('release', { ...REHEARSAL, propimages: 1, die: 'AD102' }),
    /propimages flag/,
  );
});
test('a selection without the flag is refused', () => {
  assert.throws(
    () => buildInstruction('release', { ...REHEARSAL, propimageImages: ['imga'], die: 'AD102' }),
    /without the flag/,
  );
});
test('asking on a leg that cannot carry it is refused, not ignored', () => {
  assert.throws(
    () => buildInstruction('check', { tierName: 'small', propimages: true, die: 'AD102' }),
    /propimages release only/,
  );
  assert.throws(
    () => buildInstruction('release', { ...REHEARSAL, propimages: true }),
    /propimages die/,
  );
});
test('a driver-matrix leg has no instruction (the instruction has no driver fields)', () => {
  assert.throws(
    () => buildInstruction('release', { tierName: 'large', releaseMatrix: { die: 'AD102' } }),
    /instruction release matrix/,
  );
});
test('a real driver-matrix row is refused the same way', () => {
  const row = RELEASE_MATRIX.dies.find((d) => d.die === 'AD102');
  assert.throws(
    () => buildInstruction('release', { tierName: 'large', releaseMatrix: row }),
    /instruction release matrix/,
  );
});
test("tierEnvironment is unchanged: the rehearsal leg sends exactly today's variables", () => {
  assert.deepEqual(tierEnvironment('release', REHEARSAL), {
    KF_MERGE_WINDOWS: '1',
    KF_TIER: 'large',
    KF_WINDOWS_RUNS: '2',
    KF_STAGES: 'specviewperf',
    KF_LINUX_APPS: '1',
    KF_WINAPPS: 'mid',
    KF_RELEASE_REHEARSAL: '1',
    KF_SPEC: 'lanea',
  });
  assert.equal(
    Object.keys(tierEnvironment('check', { tierName: 'small' })).some((k) => /PROPIMAGE/i.test(k)),
    false,
  );
});
test('the deployed Worker does not import the module yet', () => {
  assert.equal(read(here('../src/index.js')).includes('propimages'), false);
});

// ---------------------------------------------------------------- the allow-list decision
test('the private objects are not in the /bench-data allow-list (names)', () => {
  for (const o of PROPIMAGES.objects) assert.equal(BENCH_OBJECTS.has(o.ref), false, o.ref);
});
test('the private objects are not in the /bench-data allow-list (keys)', () => {
  const keys = new Set(BENCH_OBJECTS.values());
  for (const o of PROPIMAGES.objects) assert.equal(keys.has(o.object), false, o.object);
  assert.equal(
    [...keys].some((k) => k.startsWith('windows-proprietary/')),
    false,
  );
});

// ---------------------------------------------------------------- parity with the box and the standalone run
test('the golden instructions are what the Worker builds for the same leg', () => {
  for (const [name, options] of Object.entries(LEGS))
    assert.equal(canonicalInstruction(build(options)), golden(name), name);
});
test('the box parser reads the golden and writes the same bytes back', () => {
  for (const name of names) {
    const got = check({ mode: 'roundtrip', text: golden(name) });
    assert.equal(got.refused, undefined, name);
    assert.equal(got.canonical, golden(name), name);
  }
});
test('the box refuses an image list on a tier that is neither large nor middle', () => {
  for (const tier of ['small', 'adhoc']) {
    const doc = JSON.parse(golden('ad102-large-slot1-even'));
    doc.tier = tier;
    doc.linux_apps = null;
    doc.windows_runs = 1;
    const got = check({ mode: 'roundtrip', text: JSON.stringify(doc) });
    assert.match(got.refused, /propimages need the middle or large tier/, tier);
  }
});
test('the box accepts the image list on the second die (tier middle)', () => {
  const got = check({ mode: 'roundtrip', text: golden('ad104-middle-slot2-even') });
  assert.equal(got.refused, undefined);
  assert.deepEqual(
    got.runspec.images.map((i) => i.label),
    ['imgb'],
  );
});
test('every golden carries one image per box, or both on the one-die rehearsal', () => {
  const labels = (name) => JSON.parse(golden(name)).propimages.map((i) => i.label);
  assert.deepEqual(labels('ad102-large-slot1-even'), ['imga']);
  assert.deepEqual(labels('ad102-large-slot1-odd'), ['imgb']);
  assert.deepEqual(labels('ad104-middle-slot2-even'), ['imgb']);
  assert.deepEqual(labels('ad102-large-rehearsal'), ['imga', 'imgb']);
});
// the standalone command line the golden is equivalent to: one --image per image, the pin and its sha256, the label, one --app per row
const flagsOf = (doc) => [
  ...doc.propimages.flatMap((img) => [
    '--image',
    'prebuilt:pin:' + img.disk.ref + '#sha256=' + img.disk.sha256,
    '--label',
    img.label,
    ...Object.entries(img.apps).flatMap(([id, row]) => [
      '--app',
      id + '=' + row.mode + ',lane=' + row.lane,
    ]),
  ]),
  ...(doc.propimages_strict ? ['--strict'] : []),
];
test('the standalone command line for the same images is the same run spec', () => {
  for (const name of names) {
    const doc = JSON.parse(golden(name));
    const got = check({
      mode: 'cli',
      argv: flagsOf(doc),
      images: doc.propimages,
      strict: doc.propimages_strict,
    });
    assert.equal(got.refused, undefined, name);
    assert.deepEqual(got.rest, [], name);
    assert.deepEqual(got.cli, got.worker, name);
    assert.equal(got.worker.strict, true, name);
  }
});
test('the standalone command line without --strict is the permissive run of the same images', () => {
  const doc = JSON.parse(golden('ad102-large-slot1-even'));
  const argv = flagsOf(doc).filter((w) => w !== '--strict');
  const got = check({ mode: 'cli', argv, images: doc.propimages, strict: false });
  assert.equal(got.cli.strict, false);
  assert.deepEqual(got.cli, got.worker);
});
test('`propimage_apps.py plan` prints the same plan for the flags and for the instruction images', () => {
  const dir = mkdtempSync(join(tmpdir(), 'kf-pi-'));
  try {
    for (const name of names) {
      const doc = JSON.parse(golden(name));
      const file = join(dir, name + '.json');
      writeFileSync(
        file,
        JSON.stringify({
          schema: 'kayfabe.propimage-images/1',
          images: doc.propimages,
          strict: true,
        }),
      );
      const plan = (args) => python(REPO + 'scripts/ci/propimage_apps.py', ['plan', ...args]);
      const a = plan(['--images-file', file]);
      const b = plan(flagsOf(doc));
      assert.equal(a.status, b.status, name);
      assert.equal(a.stdout, b.stdout, name);
      assert.equal(a.stdout.trim().split('\n').length, doc.propimages.length, name);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
// A golden whose images all have a definition in the runner registry is accepted whole by the box; one with image B is refused by
// name until image B's definitions land (#446): the test asserts the refusal that is true today and the acceptance that will be.
const dryRun = (name) =>
  python(REPO + 'scripts/ci/vast_payload.py', [
    '--instruction',
    here('./fixtures/propimages/' + name + '.instruction.json'),
    '--dry-run',
  ]);
const missingApps = (name) => {
  const ids = registry();
  return JSON.parse(golden(name)).propimages.flatMap((img) =>
    Object.keys(img.apps)
      .filter((id) => !ids.includes(id))
      .map((id) => img.label + ':' + id),
  );
};
test('the box accepts the slot-1 leg (image A): dry run names the image and the plan holds the stage', () => {
  const run = dryRun('ad102-large-slot1-even');
  assert.equal(run.status, 0, run.stdout + run.stderr);
  assert.match(run.stdout, /^KF_PAYLOAD_PROPIMAGES strict=1 images=imga$/m);
  assert.match(run.stdout, /^KF_PAYLOAD_PLAN stages=.*:propimage_apps /m);
});
test('the box plans the stage on the second die (tier middle) too', () => {
  const run = dryRun('ad104-middle-slot2-even');
  const missing = missingApps('ad104-middle-slot2-even');
  if (missing.length === 0) {
    assert.equal(run.status, 0, run.stdout);
    assert.match(run.stdout, /^KF_PAYLOAD_PROPIMAGES strict=1 images=imgb$/m);
    assert.match(run.stdout, /^KF_PAYLOAD_PLAN stages=.*:propimage_apps$/m);
  } else {
    assert.equal(run.status, 1);
    assert.match(
      run.stdout,
      new RegExp('^KF_PAYLOAD_PROPIMAGES_REFUSED STRICT_UNKNOWN_APP:' + missing[0] + '$', 'm'),
    );
  }
});
test('--emit-instruction of the box writes the slot-1 golden back byte for byte', () => {
  const dir = mkdtempSync(join(tmpdir(), 'kf-pi-'));
  try {
    const out = join(dir, 'emitted.json');
    const run = python(REPO + 'scripts/ci/vast_payload.py', [
      '--instruction',
      here('./fixtures/propimages/ad102-large-slot1-even.instruction.json'),
      '--emit-instruction',
      out,
    ]);
    assert.equal(run.status, 0, run.stdout + run.stderr);
    assert.equal(read(out), golden('ad102-large-slot1-even'));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
test('the one-die rehearsal (images A and B): accepted once every image B application has a definition, refused by name until then', () => {
  const run = dryRun('ad102-large-rehearsal');
  const missing = missingApps('ad102-large-rehearsal');
  if (missing.length === 0) {
    assert.equal(run.status, 0, run.stdout);
    assert.match(run.stdout, /^KF_PAYLOAD_PROPIMAGES strict=1 images=imga,imgb$/m);
  } else {
    assert.equal(run.status, 1);
    assert.match(
      run.stdout,
      new RegExp('^KF_PAYLOAD_PROPIMAGES_REFUSED STRICT_UNKNOWN_APP:' + missing[0] + '$', 'm'),
    );
  }
});
// The environment path: what the box builds from the Worker's environment equals the Worker's instruction without the part
// (the part has no environment spelling: the Worker sends it as the instruction file).
const BASE_LEGS = {
  'rehearsal, spec lane a': ['release', REHEARSAL],
  'rehearsal, spec lane both': ['release', { ...REHEARSAL, spec: 'both' }],
  'rehearsal, no spec': [
    'release',
    { tierName: 'large', windowsRuns: 2, linuxApps: true, winApps: 'mid', rehearsal: true },
  ],
  'large, one windows run': ['release', { tierName: 'large', linuxApps: true, winApps: 'mid' }],
  'release middle leg': ['release', { tierName: 'middle', linuxApps: true, winApps: 'mid' }],
  'check middle': ['check', { tierName: 'middle', linuxApps: true, winApps: 'mid' }],
  'check small': ['check', { tierName: 'small' }],
  sweep: ['sweep', { tierName: 'small' }],
};
test('the box builds from the Worker environment the instruction the Worker builds (every base leg)', () => {
  const dir = mkdtempSync(join(tmpdir(), 'kf-pi-'));
  try {
    for (const [name, [profile, options]] of Object.entries(BASE_LEGS)) {
      const out = join(dir, 'emitted.json');
      const run = python(
        REPO + 'scripts/ci/vast_payload.py',
        ['--emit-instruction', out],
        tierEnvironment(profile, options),
      );
      assert.equal(run.status, 0, name + '\n' + run.stdout + run.stderr);
      assert.equal(read(out), canonicalInstruction(buildInstruction(profile, options)), name);
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
test('the environment adapter reads the variables the way the box does', () => {
  assert.deepEqual(instructionFromEnvironment({}), {
    tier: 'unknown',
    windows: true,
    windows_runs: 1,
    stages: [],
    spec: null,
    winapps: null,
    linux_apps: null,
    rehearsal: false,
  });
  assert.equal(instructionFromEnvironment({ KF_MERGE_WINDOWS: '0' }).windows, false);
  assert.equal(instructionFromEnvironment({ KF_WINDOWS_RUNS: '2' }).windows_runs, 2);
  assert.deepEqual(instructionFromEnvironment({ KF_STAGES: 'a:b' }).stages, ['a', 'b']);
});
