// The MATLAB image in the Worker's configuration and in the instruction it builds (src/propimages.js: linux_images, matlabPlan; the box side is
// scripts/ci/matlab_stage.py and the `matlab` field of scripts/ci/payload_instruction.py). GPU-free, no network, nothing rented.
// FALSIFIER (written before the code): the part the Worker builds is refused or read differently by the box; the Linux image is tied to the Windows
// slot rotation; a config that names a Windows object, an unknown row or a bad lane for it is accepted; or the part is emitted without being asked for.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  PROPIMAGES,
  MATLAB_ROWS,
  propimagesConfigProblems,
  matlabPlan,
  buildLeg,
  buildInstruction,
  canonicalInstruction,
  propimagePlan,
} from '../src/propimages.js';
import { RELEASE_MATRIX } from '../src/release-matrix.js';
import { estimateBandwidth } from '../src/bandwidth.js';

const here = (p) => new URL(p, import.meta.url).pathname;
const REPO = here('../../../');
const GOLDEN = here('./fixtures/matlab/ad102-large-matlab.instruction.json');
const LEG = {
  tierName: 'large',
  windowsRuns: 2,
  winApps: 'mid',
  linuxApps: true,
  matlab: true,
  die: 'AD102',
};
const python = (script, args) =>
  spawnSync('python3', ['-B', '-I', REPO + script, ...args], {
    encoding: 'utf8',
    timeout: 60000,
    cwd: REPO,
    env: { PATH: process.env.PATH },
  });
const edit = (fn) => {
  const c = structuredClone(PROPIMAGES);
  fn(c);
  return c;
};
const refuses = (c, token) => {
  const problems = propimagesConfigProblems(c);
  assert.ok(problems.includes(token), token + ' not in ' + JSON.stringify(problems));
};
const linux = (c) => c.linux_images.find((i) => i.label === 'matlab');
const withTemp = (fn) => {
  const dir = mkdtempSync(join(tmpdir(), 'kf-matlab-'));
  try {
    return fn(dir);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
};
const dry = (doc, extra = []) =>
  withTemp((dir) => {
    const file = join(dir, 'i.json');
    writeFileSync(file, canonicalInstruction(doc));
    return python('scripts/ci/vast_payload.py', ['--instruction', file, '--dry-run', ...extra]);
  });

test('the committed configuration holds the MATLAB image: the object, every row in the discovery lane, no per-family list', () => {
  assert.deepEqual(propimagesConfigProblems(), []);
  const m = linux(PROPIMAGES);
  const o = PROPIMAGES.objects.find((x) => x.id === m.object);
  assert.equal(o.role, 'linux_image');
  assert.equal(o.object, 'linux-proprietary/kfimg-matlab-20261117/image.zip');
  assert.equal(o.size, 8128391922);
  assert.equal(o.sha256, '30780fbd7762690a86cda017aeacd459fa5ef5ed40aca6d6686e43b0234a9f1a');
  assert.equal(m.families, 'generic');
  assert.deepEqual(Object.keys(m.rows), [...MATLAB_ROWS]);
  assert.ok(Object.values(m.rows).every((r) => r.lane === 'discovery'));
  assert.equal(m.measured_families, undefined);
});
test('the row ids are the box own table (scripts/bench/linux/matlab_rows.py plan)', () => {
  const run = python('scripts/bench/linux/matlab_rows.py', ['plan']);
  assert.equal(run.status, 0, run.stderr);
  assert.deepEqual(
    run.stdout
      .trim()
      .split('\n')
      .map((l) => l.split(/\s+/)[0]),
    [...MATLAB_ROWS],
  );
});
test('the Worker object equals the pin the box reads (ref, size, sha256)', () => {
  const pin = JSON.parse(readFileSync(REPO + 'scripts/ci/matlab_pins.json', 'utf8')).items[0];
  const o = PROPIMAGES.objects.find((x) => x.id === 'imgc');
  assert.deepEqual([pin.ref, pin.size, pin.sha256], [o.ref, o.size, o.sha256]);
});
test('a Linux image naming a Windows image object is refused', () => {
  refuses(
    edit((c) => (linux(c).object = 'imga')),
    'LINUX_OBJECT:matlab',
  );
  refuses(
    edit((c) => (linux(c).object = 'imgb')),
    'LINUX_OBJECT:matlab',
  );
});
test('a Linux image naming the signed GOP or a missing object is refused', () => {
  refuses(
    edit((c) => (linux(c).object = 'gop')),
    'LINUX_OBJECT:matlab',
  );
  refuses(
    edit((c) => (linux(c).object = 'nope')),
    'LINUX_OBJECT:matlab',
  );
});
test('a Linux image row with a bad lane is refused', () => {
  refuses(
    edit((c) => (linux(c).rows.gpu_fft.lane = 'maybe')),
    'LANE:matlab:gpu_fft',
  );
  refuses(
    edit((c) => delete linux(c).rows.gpu_fft.lane),
    'LANE:matlab:gpu_fft',
  );
});
test('a Linux image row the box does not know is refused', () => {
  refuses(
    edit((c) => (linux(c).rows.nonsense = { lane: 'gate' })),
    'ROW_ID:matlab:nonsense',
  );
});
test('a Linux image row id that is path-like or upper case is refused', () => {
  refuses(
    edit((c) => (linux(c).rows['../gpu_fft'] = { lane: 'gate' })),
    'ROW_ID:matlab:../gpu_fft',
  );
  refuses(
    edit((c) => (linux(c).rows.GPU_FFT = { lane: 'gate' })),
    'ROW_ID:matlab:GPU_FFT',
  );
});
test('a Linux image with a second family list, an unknown isolation or no scheduling note is refused', () => {
  refuses(
    edit((c) => (linux(c).families = ['ada'])),
    'LINUX_FAMILIES:matlab',
  );
  refuses(
    edit((c) => (linux(c).isolation = 'none')),
    'LINUX_ISOLATION:matlab',
  );
  refuses(
    edit((c) => (linux(c).scheduling = '')),
    'LINUX_SCHEDULING:matlab',
  );
});
test('a Linux image with no rows, an extra key, a clashing label, bad families or an unpinned object is refused', () => {
  refuses(
    edit((c) => (linux(c).rows = {})),
    'NO_ROWS:matlab',
  );
  refuses(
    edit((c) => (linux(c).bogus = 1)),
    'LINUX_IMAGE_SCHEMA',
  );
  refuses(
    edit((c) => (linux(c).label = 'imga')),
    'DUPLICATE_LABEL:imga',
  );
  refuses(
    edit((c) => (linux(c).measured_families = ['moon'])),
    'MEASURED_FAMILIES:matlab',
  );
  refuses(
    edit((c) => (c.objects.find((o) => o.id === 'imgc').sha256 = 'x')),
    'UNPINNED:imgc',
  );
  refuses(
    edit((c) => (c.linux_images = [])),
    'LINUX_IMAGES',
  );
});
test('a Windows image cannot take the Linux object, and the slot rule is untouched', () => {
  refuses(
    edit((c) => (c.images[0].object = 'imgc')),
    'UNKNOWN_OBJECT:imga',
  );
  const before = propimagePlan({
    profile: 'release',
    tierName: 'large',
    die: 'AD102',
    releaseSeq: 2,
  });
  assert.deepEqual(before.assignment.images, ['imga']);
  const odd = propimagePlan({ profile: 'release', tierName: 'large', die: 'AD102', releaseSeq: 3 });
  assert.deepEqual(odd.assignment.images, ['imgb']);
});
test('the part: a pinned image, the die guest driver, every row in the discovery lane, no strict switch, no slot', () => {
  const { part, lines } = matlabPlan({ profile: 'release', tierName: 'large', die: 'AD102' });
  assert.deepEqual(Object.keys(part), ['matlab']);
  assert.deepEqual(part.matlab.image, {
    from: 'pin',
    ref: 'propimage-kfimg-matlab-20261117.zip',
    sha256: '30780fbd7762690a86cda017aeacd459fa5ef5ed40aca6d6686e43b0234a9f1a',
  });
  assert.deepEqual(part.matlab.driver, { version: '580.173.02' });
  assert.equal(part.matlab.isolation, 'row');
  assert.deepEqual(Object.keys(part.matlab.rows), [...MATLAB_ROWS]);
  assert.ok(Object.values(part.matlab.rows).every((r) => r.lane === 'discovery'));
  assert.equal(JSON.stringify(part).includes('slot'), false);
  assert.equal(JSON.stringify(part).includes('strict'), false);
  assert.match(lines[0], /matlab on AD102 \(ada\): guest driver 580\.173\.02/);
  assert.match(lines[0], /not part of the A\/B slot rotation/);
});
test('the driver is the die own pinned guest driver from the release matrix', () => {
  for (const die of ['TU106', 'GA102', 'AD102', 'GB202']) {
    const row = RELEASE_MATRIX.dies.find((d) => d.die === die);
    assert.equal(
      matlabPlan({ profile: 'release', tierName: 'large', die }).part.matlab.driver.version,
      row.drivers.linux_guest_driver,
    );
  }
});
test('refusals: not a release, not the large tier, a die with an unpinned driver, an unknown die, an unknown image', () => {
  assert.throws(
    () => matlabPlan({ profile: 'check', tierName: 'large', die: 'AD102' }),
    /matlab release only/,
  );
  assert.throws(
    () => matlabPlan({ profile: 'release', tierName: 'middle', die: 'AD104' }),
    /matlab tier/,
  );
  assert.throws(
    () => matlabPlan({ profile: 'release', tierName: 'large', die: 'AD104' }),
    /matlab driver:AD104/,
  );
  assert.throws(
    () => matlabPlan({ profile: 'release', tierName: 'large', die: 'XX999' }),
    /matlab die/,
  );
  assert.throws(
    () => matlabPlan({ profile: 'release', tierName: 'large', die: 'AD102', label: 'nope' }),
    /matlab unknown image/,
  );
});
test('a family the image has run on keeps its configured lanes; every other family is discovery', () => {
  const c = edit((x) => {
    linux(x).measured_families = ['ada'];
    linux(x).rows.gpu_matmul.lane = 'gate';
  });
  assert.equal(
    matlabPlan({ profile: 'release', tierName: 'large', die: 'AD102' }, { config: c }).part.matlab
      .rows.gpu_matmul.lane,
    'gate',
  );
  assert.equal(
    matlabPlan({ profile: 'release', tierName: 'large', die: 'GA102' }, { config: c }).part.matlab
      .rows.gpu_matmul.lane,
    'discovery',
  );
});
test('an unmeasured family never gets a gate lane, whatever the configuration says', () => {
  const c = edit((x) => (linux(x).rows.gpu_matmul.lane = 'gate'));
  const part = matlabPlan(
    { profile: 'release', tierName: 'large', die: 'AD102' },
    { config: c },
  ).part;
  assert.ok(Object.values(part.matlab.rows).every((r) => r.lane === 'discovery'));
});
test('the second die (a middle leg) is refused by the builder too, with a driver that is pinned', () => {
  assert.throws(
    () => matlabPlan({ profile: 'release', tierName: 'middle', die: 'TU116' }),
    /matlab tier/,
  );
  assert.throws(
    () =>
      buildInstruction('release', {
        tierName: 'middle',
        winApps: 'mid',
        linuxApps: true,
        matlab: true,
        die: 'TU116',
      }),
    /matlab tier/,
  );
});
test('the part is emitted only when asked for (default off) and never changes the base instruction', () => {
  const { matlab, ...base } = LEG;
  const off = buildInstruction('release', base);
  assert.equal('matlab' in off, false);
  const on = buildInstruction('release', LEG);
  const { matlab: part, ...rest } = on;
  assert.deepEqual(rest, off);
  assert.equal(part.label, 'matlab');
  assert.throws(() => buildInstruction('release', { ...LEG, matlab: 'yes' }), /matlab flag/);
  assert.throws(
    () => buildInstruction('check', { tierName: 'small', matlab: true, die: 'AD102' }),
    /matlab release only/,
  );
});
test('both parts can be asked for in one leg: the Windows split and the Linux image are independent', () => {
  const both = buildLeg('release', { ...LEG, propimages: true, releaseSeq: 2 });
  assert.ok(both.instruction.propimages.length >= 1);
  assert.ok(both.instruction.matlab);
  assert.ok(both.plan.lines.some((l) => l.includes('RUNS HERE')) || both.plan.lines.length > 1);
  assert.ok(both.plan.lines.some((l) => l.startsWith('matlab on AD102')));
});
test('the instruction the Worker builds equals the committed golden, byte for byte', () => {
  assert.equal(
    canonicalInstruction(buildInstruction('release', LEG)),
    readFileSync(GOLDEN, 'utf8'),
  );
});
test('the box reads the golden and writes the same bytes back; its dry run plans the stage and names the object', () => {
  withTemp((dir) => {
    const out = join(dir, 'out.json');
    const run = python('scripts/ci/vast_payload.py', [
      '--instruction',
      GOLDEN,
      '--emit-instruction',
      out,
    ]);
    assert.equal(run.status, 0, run.stdout + run.stderr);
    assert.equal(readFileSync(out, 'utf8'), readFileSync(GOLDEN, 'utf8'));
  });
  const run = python('scripts/ci/vast_payload.py', ['--instruction', GOLDEN, '--dry-run']);
  assert.equal(run.status, 0, run.stdout + run.stderr);
  assert.match(run.stdout, /^KF_PAYLOAD_MATLAB strict=1 rows=8 driver=580\.173\.02$/m);
  assert.match(run.stdout, /^KF_PAYLOAD_PLAN stages=.*:matlab planned=.*:matlab$/m);
});
test('the box refuses the part by name: a changed sha256, an unpinned driver, an unknown row, the middle tier, a flag beside it', () => {
  const doc = JSON.parse(readFileSync(GOLDEN, 'utf8'));
  const mutate = (fn) => {
    const d = structuredClone(doc);
    fn(d);
    return d;
  };
  const cases = [
    [
      mutate((d) => (d.matlab.image.sha256 = 'a'.repeat(64))),
      /KF_PAYLOAD_MATLAB_REFUSED PIN_SHA256_MISMATCH/,
    ],
    [
      mutate((d) => (d.matlab.image.ref = 'propimage-other.zip')),
      /KF_PAYLOAD_MATLAB_REFUSED PIN_UNKNOWN/,
    ],
    [
      mutate((d) => (d.matlab.driver.version = '9.9.9')),
      /KF_PAYLOAD_MATLAB_REFUSED DRIVER_UNPINNED:9\.9\.9/,
    ],
    [
      mutate((d) => (d.matlab.rows.nonsense = { lane: 'gate' })),
      /KF_PAYLOAD_MATLAB_REFUSED ROW_UNKNOWN:nonsense/,
    ],
    [mutate((d) => (d.matlab.rows = {})), /KF_PAYLOAD_MATLAB_REFUSED NO_ROWS/],
  ];
  for (const [d, pattern] of cases) {
    const run = dry(d);
    assert.equal(run.status, 1, JSON.stringify(d.matlab).slice(0, 80));
    assert.match(run.stdout, pattern);
  }
  const middle = dry(mutate((d) => (d.tier = 'middle')));
  assert.equal(middle.status, 1);
  assert.match(middle.stdout, /KF_PAYLOAD_INSTRUCTION_REFUSED/);
  const flag = dry(doc, ['--matlab-image', '/tmp/x.zip', '--only', 'matlab']);
  assert.match(flag.stdout, /KF_PAYLOAD_MATLAB_REFUSED INSTRUCTION_AND_IMAGE_FLAG/);
  const extra = dry(mutate((d) => (d.matlab.bogus = 1)));
  assert.equal(extra.status, 1);
  assert.match(extra.stdout, /KF_PAYLOAD_INSTRUCTION_REFUSED/);
});
test('bandwidth: the stage is planned from the instruction and charges the pinned image size plus the installer', () => {
  const doc = JSON.parse(readFileSync(GOLDEN, 'utf8'));
  const est = estimateBandwidth(doc, doc.tier);
  assert.ok(est.stages.includes('matlab'));
  const image = est.items.find((i) => i.id === 'matlab.image');
  assert.equal(image.down, 8128391922);
  assert.equal(image.from, 'propimages.json:imgc');
  assert.equal(est.items.find((i) => i.id === 'matlab.driver').down, 398000000);
  const { matlab, ...base } = doc;
  assert.equal(estimateBandwidth(base, base.tier).stages.includes('matlab'), false);
  assert.ok(estimateBandwidth(base, base.tier, { only: ['matlab'] }).stages.includes('matlab'));
});
test('the Python twin of the bandwidth table selects the same lines for the matlab leg', () => {
  withTemp((dir) => {
    const doc = JSON.parse(readFileSync(GOLDEN, 'utf8'));
    const file = join(dir, 'm.json');
    writeFileSync(file, JSON.stringify(doc));
    const run = python('scripts/ci/bandwidth_check.py', ['--explain', file, '--tier', doc.tier]);
    assert.equal(run.status, 0, run.stderr);
    const js = estimateBandwidth(doc, doc.tier).items.map((i) => ({
      id: i.id,
      stage: i.stage,
      label: i.label ?? null,
      down: i.down,
      up: i.up,
    }));
    assert.deepEqual(JSON.parse(run.stdout), js);
  });
});
