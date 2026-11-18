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
  compareDottedVersions,
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
test('the committed image carries the minimum guest driver MATLAB needs, a pinned Linux installer, with its measured basis', () => {
  const m = linux(PROPIMAGES);
  assert.equal(m.min_guest_driver, '590.48.01');
  assert.ok(
    RELEASE_MATRIX.driver_installers.some(
      (p) => p.os === 'linux' && p.version === m.min_guest_driver && p.status === 'PINNED',
    ),
  );
  assert.match(m.min_guest_driver_basis, /traces\/matlab_gpu_20261117/);
  assert.match(m.min_guest_driver_basis, /MEASURED/);
  assert.match(m.min_guest_driver_basis, /580\.159\.04/);
});
test('the committed image carries the binary the image keeps matlab at, with the measured basis (2026-11-18: it is not on the guest PATH)', () => {
  const m = linux(PROPIMAGES);
  assert.equal(m.bin, '/home/kfimage/MATLAB/R2026b/bin/matlab');
  assert.match(m.bin_basis, /^MEASURED 2026-11-18/);
  assert.match(m.bin_basis, /inspection boot/);
  assert.match(m.bin_basis, /not on the guest's ssh PATH/);
  assert.match(m.bin_basis, /find/);
  assert.deepEqual(propimagesConfigProblems(), []);
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
test('the minimum guest driver is optional', () => {
  const c = edit((x) => {
    delete linux(x).min_guest_driver;
    delete linux(x).min_guest_driver_basis;
  });
  assert.deepEqual(propimagesConfigProblems(c), []);
});
test('a minimum guest driver that is not a dotted version is refused', () => {
  for (const bad of ['abc', '590', '590.48.01.2.3', ' 590.48.01', '', 590.48, null, ['590.48.01']])
    refuses(
      edit((c) => (linux(c).min_guest_driver = bad)),
      'MIN_GUEST_DRIVER:matlab',
    );
});
test('a minimum guest driver that is dotted but not a PINNED Linux installer of the release matrix is refused', () => {
  // listed and UNPINNED, absent from the matrix, or a Windows installer
  for (const bad of ['580.94.02', '9.9.9', '580.97'])
    refuses(
      edit((c) => (linux(c).min_guest_driver = bad)),
      'MIN_GUEST_DRIVER:matlab',
    );
  // the check is against the matrix handed in: a matrix without the pin refuses the committed value
  const m = structuredClone(RELEASE_MATRIX);
  m.driver_installers = m.driver_installers.filter((p) => p.version !== '590.48.01');
  assert.ok(
    propimagesConfigProblems(PROPIMAGES, { releaseMatrix: m }).includes('MIN_GUEST_DRIVER:matlab'),
  );
});
test('the plan refuses a configuration whose minimum is not a pinned installer, with the configuration error, before it builds anything', () => {
  const c = edit((x) => (linux(x).min_guest_driver = '9.9.9'));
  assert.throws(
    () => matlabPlan({ profile: 'release', tierName: 'large', die: 'AD102' }, { config: c }),
    /propimages config: MIN_GUEST_DRIVER:matlab/,
  );
});
test('a minimum guest driver with no basis, or an empty one, is refused', () => {
  refuses(
    edit((c) => delete linux(c).min_guest_driver_basis),
    'MIN_GUEST_DRIVER_BASIS:matlab',
  );
  refuses(
    edit((c) => (linux(c).min_guest_driver_basis = '')),
    'MIN_GUEST_DRIVER_BASIS:matlab',
  );
});
test('a basis with no minimum is a stray key, not a free note, and is refused', () => {
  refuses(
    edit((c) => delete linux(c).min_guest_driver),
    'MIN_GUEST_DRIVER_BASIS:matlab',
  );
});
// ---- `bin` (2026-11-18): where the image keeps the matlab binary in the guest; an image property, so config, never code
test('the binary is optional: without it (and its basis) the configuration is valid and the part carries no `bin`', () => {
  const c = edit((x) => {
    delete linux(x).bin;
    delete linux(x).bin_basis;
  });
  assert.deepEqual(propimagesConfigProblems(c), []);
  const part = matlabPlan(LARGE('AD102'), { config: c }).part;
  assert.equal('bin' in part.matlab, false);
  assert.equal(JSON.stringify(part).includes('"bin"'), false);
});
test('a binary that is not the box pattern (letters, digits and _ . / -, 1 to 120 characters, a string) is refused', () => {
  const long = 'a'.repeat(121);
  for (const bad of [
    '',
    long,
    'matlab -batch',
    'matlab;id',
    '$(id)',
    '`id`',
    'a|b',
    'a&b',
    "a'b",
    'a\nb',
    'matlab\n',
    ' matlab',
    'mat\u0142ab',
    5,
    null,
    true,
    ['matlab'],
    {},
  ])
    refuses(
      edit((c) => (linux(c).bin = bad)),
      'BIN:matlab',
    );
});
test('a binary that is the box pattern is accepted: a bare name, a path, the longest allowed', () => {
  for (const good of [
    'matlab',
    '/home/kfimage/MATLAB/R2026b/bin/matlab',
    'bin/matlab-2026b_x.1',
    'a'.repeat(120),
  ])
    assert.deepEqual(propimagesConfigProblems(edit((c) => (linux(c).bin = good))), [], good);
});
test('the binary needs its basis: a missing, an empty or a non-string one is refused', () => {
  refuses(
    edit((c) => delete linux(c).bin_basis),
    'BIN_BASIS:matlab',
  );
  refuses(
    edit((c) => (linux(c).bin_basis = '')),
    'BIN_BASIS:matlab',
  );
  refuses(
    edit((c) => (linux(c).bin_basis = 5)),
    'BIN_BASIS:matlab',
  );
});
test('a basis with no binary is a stray key, not a free note, and is refused', () => {
  refuses(
    edit((c) => delete linux(c).bin),
    'BIN_BASIS:matlab',
  );
});
test('the binary and its basis are keys of the Linux image schema, not stray keys', () => {
  const problems = propimagesConfigProblems();
  assert.equal(problems.includes('LINUX_IMAGE_SCHEMA'), false);
  const c = edit((x) => (linux(x).binary = '/x/matlab'));
  refuses(c, 'LINUX_IMAGE_SCHEMA');
});
test('the plan copies the configured binary into the part, as it is', () => {
  const part = matlabPlan(LARGE('AD102')).part;
  assert.equal(part.matlab.bin, '/home/kfimage/MATLAB/R2026b/bin/matlab');
  const c = edit((x) => (linux(x).bin = '/opt/other/bin/matlab'));
  assert.equal(matlabPlan(LARGE('AD102'), { config: c }).part.matlab.bin, '/opt/other/bin/matlab');
});
test('the plan refuses a configuration with a bad binary with the configuration error, before it builds anything', () => {
  const c = edit((x) => (linux(x).bin = 'matlab;id'));
  assert.throws(() => matlabPlan(LARGE('AD102'), { config: c }), /propimages config: BIN:matlab/);
});
test('the keys of the matlab part are pinned: the binary is one of them, every other key is as before', () => {
  const { part } = matlabPlan(LARGE('AD102'));
  assert.deepEqual(Object.keys(part.matlab).sort(), [
    'bin',
    'driver',
    'image',
    'isolation',
    'label',
    'rows',
  ]);
  const c = edit((x) => {
    delete linux(x).bin;
    delete linux(x).bin_basis;
  });
  assert.deepEqual(Object.keys(matlabPlan(LARGE('AD102'), { config: c }).part.matlab).sort(), [
    'driver',
    'image',
    'isolation',
    'label',
    'rows',
  ]);
});
test('the box refuses a binary the Worker pattern would also refuse, by the schema (the two patterns agree)', () => {
  const doc = JSON.parse(readFileSync(GOLDEN, 'utf8'));
  assert.equal(doc.matlab.bin, matlabPlan(LARGE('AD102')).part.matlab.bin);
  for (const bad of ['matlab;id', '', 'a'.repeat(121), 'a b', 'matlab\n']) {
    const d = structuredClone(doc);
    d.matlab.bin = bad;
    const run = dry(d);
    assert.equal(run.status, 1, JSON.stringify(bad));
    assert.match(run.stdout, /KF_PAYLOAD_INSTRUCTION_REFUSED/);
    assert.equal(
      propimagesConfigProblems(edit((c) => (linux(c).bin = bad))).includes('BIN:matlab'),
      true,
      JSON.stringify(bad),
    );
  }
  for (const good of ['matlab', 'a'.repeat(120), '/opt/x/bin/matlab']) {
    const d = structuredClone(doc);
    d.matlab.bin = good;
    const run = dry(d);
    assert.equal(run.status, 0, good + run.stdout);
  }
});
test('dotted versions compare field by field as numbers, a missing field is zero', () => {
  assert.ok(compareDottedVersions('580.173.02', '590.48.01') < 0);
  assert.ok(compareDottedVersions('590.48.01', '580.173.02') > 0);
  assert.ok(compareDottedVersions('1000.2.3', '590.48.01') > 0); // as text it is smaller
  assert.ok(compareDottedVersions('590.9', '590.10') < 0); // as text it is larger
  assert.equal(compareDottedVersions('590.48.01', '590.48.1'), 0);
  assert.equal(compareDottedVersions('590.48', '590.48.0'), 0);
  assert.ok(compareDottedVersions('590.48', '590.48.1') < 0);
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
  assert.deepEqual(part.matlab.driver, { version: '590.48.01' });
  assert.equal(part.matlab.isolation, 'row');
  assert.deepEqual(Object.keys(part.matlab.rows), [...MATLAB_ROWS]);
  assert.ok(Object.values(part.matlab.rows).every((r) => r.lane === 'discovery'));
  assert.equal(JSON.stringify(part).includes('slot'), false);
  assert.equal(JSON.stringify(part).includes('strict'), false);
  assert.match(lines[0], /matlab on AD102 \(ada\): guest driver 590\.48\.01 /);
  assert.match(
    lines[0],
    /\(the image needs at least 590\.48\.01; the die's default 580\.173\.02 is older\)/,
  );
  assert.match(lines[0], /not part of the A\/B slot rotation/);
});
const LARGE = (die) => ({ profile: 'release', tierName: 'large', die });
// a release matrix in which the die AD102 has the given default guest driver and the given Linux installers are PINNED
const matrixWith = (defaultDriver, pinned, extra = []) => {
  const m = structuredClone(RELEASE_MATRIX);
  m.dies.find((d) => d.die === 'AD102').drivers.linux_guest_driver = defaultDriver;
  const real = new Map(m.driver_installers.map((p) => [p.os + '/' + p.version, p]));
  m.driver_installers = [...pinned, ...extra].map((v) => {
    const base = real.get('linux/580.173.02');
    return { ...base, version: v, status: 'PINNED', size: 400000000 + Number(v.split('.')[0]) };
  });
  return m;
};
test('the driver is the die own pinned guest driver from the release matrix, raised to the image minimum when older', () => {
  const min = linux(PROPIMAGES).min_guest_driver;
  for (const die of ['TU106', 'GA102', 'AD102', 'GB202']) {
    const row = RELEASE_MATRIX.dies.find((d) => d.die === die);
    const older = row.drivers.linux_guest_driver.split('.')[0] < min.split('.')[0];
    assert.equal(
      matlabPlan(LARGE(die)).part.matlab.driver.version,
      older ? min : row.drivers.linux_guest_driver,
      die,
    );
  }
});
test('a die whose default guest driver is older than the image minimum gets the pinned minimum, never anything higher, and the line says why', () => {
  // 580.173.02 < 590.48.01 < 595.10.01: all three pinned, the minimum is picked, not the newest
  const releaseMatrix = matrixWith('580.173.02', ['580.173.02', '590.48.01', '595.10.01']);
  const { part, lines } = matlabPlan(LARGE('AD102'), { releaseMatrix });
  assert.deepEqual(part.matlab.driver, { version: '590.48.01' });
  assert.equal(
    lines[0].includes(
      "guest driver 590.48.01 (the image needs at least 590.48.01; the die's default 580.173.02 is older)",
    ),
    true,
    lines[0],
  );
});
test('versions are compared as dotted numbers, not as text: 580.173.02 is older than 590.48.01, 600.1.0 and 1000.2.3 are not', () => {
  const pinned = ['580.173.02', '590.48.01', '600.1.0', '1000.2.3'];
  const raised = matlabPlan(LARGE('AD102'), { releaseMatrix: matrixWith('580.173.02', pinned) });
  assert.equal(raised.part.matlab.driver.version, '590.48.01');
  // as text '600.1.0' < '590.48.01' is false but '1000.2.3' < '590.48.01' is true: a text compare would raise it
  for (const v of ['600.1.0', '1000.2.3']) {
    const kept = matlabPlan(LARGE('AD102'), { releaseMatrix: matrixWith(v, pinned) });
    assert.equal(kept.part.matlab.driver.version, v);
  }
});
test('a default that is newer by number but older as text (590.100.01 against 590.48.01) is left alone', () => {
  const pinned = ['590.48.01', '590.100.01'];
  const { part } = matlabPlan(LARGE('AD102'), { releaseMatrix: matrixWith('590.100.01', pinned) });
  assert.deepEqual(part.matlab.driver, { version: '590.100.01' });
});
test('a missing field counts as zero: 590.48 is older than 590.48.01, 590.48.1 is the same number as 590.48.01', () => {
  const pinned = ['590.48', '590.48.01', '590.48.1'];
  const short = matlabPlan(LARGE('AD102'), { releaseMatrix: matrixWith('590.48', pinned) });
  assert.deepEqual(short.part.matlab.driver, { version: '590.48.01' });
  const same = matlabPlan(LARGE('AD102'), { releaseMatrix: matrixWith('590.48.1', pinned) });
  assert.deepEqual(same.part.matlab.driver, { version: '590.48.1' });
});
test('a die whose default guest driver is already at or above the image minimum is left alone, with no raise line', () => {
  const pinned = ['580.173.02', '590.48.01', '595.10.01'];
  for (const v of ['590.48.01', '595.10.01']) {
    const { part, lines } = matlabPlan(LARGE('AD102'), { releaseMatrix: matrixWith(v, pinned) });
    assert.deepEqual(part.matlab.driver, { version: v });
    assert.match(
      lines[0],
      new RegExp('matlab on AD102 \\(ada\\): guest driver ' + v.replace(/\./g, '\\.') + ', '),
    );
    assert.equal(lines[0].includes('older'), false, lines[0]);
    assert.equal(lines[0].includes('needs at least'), false, lines[0]);
  }
});
test('an image with no minimum takes the die default as before, whatever it is', () => {
  const c = edit((x) => {
    delete linux(x).min_guest_driver;
    delete linux(x).min_guest_driver_basis;
  });
  const { part, lines } = matlabPlan(LARGE('AD102'), { config: c });
  assert.deepEqual(part.matlab.driver, { version: '580.173.02' });
  assert.equal(lines[0].includes('needs at least'), false, lines[0]);
});
test('a minimum absent from the release matrix in use is refused by name, whether the die default is older or above it', () => {
  // the committed config is valid against the committed matrix; the matrix handed to the plan lacks the minimum
  const missing = matrixWith('580.173.02', ['580.173.02']);
  assert.throws(
    () => matlabPlan(LARGE('AD102'), { releaseMatrix: missing }),
    /matlab min driver:590\.48\.01/,
  );
  const above = matrixWith('595.10.01', ['595.10.01']);
  assert.throws(
    () => matlabPlan(LARGE('AD102'), { releaseMatrix: above }),
    /matlab min driver:590\.48\.01/,
  );
});
test('a minimum that is listed but UNPINNED in the release matrix in use is refused by name', () => {
  const unpinned = matrixWith('580.173.02', ['580.173.02', '590.48.01']);
  unpinned.driver_installers.find((p) => p.version === '590.48.01').status = 'UNPINNED';
  assert.throws(
    () => matlabPlan(LARGE('AD102'), { releaseMatrix: unpinned }),
    /matlab min driver:590\.48\.01/,
  );
});
test('a minimum that is only a Windows installer of that number is refused by name', () => {
  const windows = matrixWith('580.173.02', ['580.173.02', '590.48.01']);
  windows.driver_installers.find((p) => p.version === '590.48.01').os = 'windows';
  assert.throws(
    () => matlabPlan(LARGE('AD102'), { releaseMatrix: windows }),
    /matlab min driver:590\.48\.01/,
  );
});
test('a die whose own default driver is not pinned is still refused by name, before the minimum is considered', () => {
  const m = matrixWith('580.173.02', ['590.48.01']);
  assert.throws(() => matlabPlan(LARGE('AD102'), { releaseMatrix: m }), /matlab driver:AD102/);
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
  assert.match(
    run.stdout,
    new RegExp('^KF_PAYLOAD_MATLAB strict=1 rows=' + MATLAB_ROWS.length + ' driver=590\\.48\\.01$', 'm'),
  );
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
  // the installer the instruction carries is the pinned 590.48.01 (the image minimum): the table estimate is its pinned size
  const pinned = RELEASE_MATRIX.driver_installers.find(
    (p) => p.os === 'linux' && p.version === doc.matlab.driver.version,
  );
  assert.equal(doc.matlab.driver.version, '590.48.01');
  assert.equal(est.items.find((i) => i.id === 'matlab.driver').down, pinned.size);
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
