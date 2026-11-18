// The Worker's knowledge of the proprietary Windows images (docs/design/V3_WORKER_PROPIMAGES.md).
// STATUS: DESIGN-ONLY, 2026-11-17. Pure and tested. src/index.js does not import this file; src/legs.js does, and runs only behind
// PROPIMAGES_ENABLED (default off), so no rental changes until the owner switches it on. The deployed Worker is unchanged. Owner decision 2026-11-16: the Worker knows image A and image B from
// configuration (propimages.json), with the applications on them, and builds the instruction from it; the Worker/CI run is
// STRICT (everything in the configuration must run, zero skips); an application is in the gate lane (must pass) or the
// discovery lane (may fail; the failure is a listed finding), and standalone runs take the same instruction shape.
//
// The output is the `propimages` / `propimages_strict` part of the payload instruction (schema kayfabe.payload-instruction/1,
// ci/vast-broker/planner/schemas/instruction.schema.json) in exactly the shape scripts/ci/vast_payload.py --instruction and the
// generic runner (scripts/bench/windows/propimage_runspec.py) read. test/propimages.test.js proves the shape against both.
import config from '../propimages.json' with { type: 'json' };
import { DEFAULTS } from './defaults.js';
import { tierEnvironment, familyOf } from './tier.js';
import { RELEASE_MATRIX } from './release-matrix.js';

export const PROPIMAGES = config;
export const INSTRUCTION_SCHEMA = 'kayfabe.payload-instruction/1';
// Images are for the RELEASE run only (owner, 2026-11-16): profile `release`, never the check profile's small or middle tier and
// never the sweep. Inside a release the two dies of a family run tier `large` (slot 1, the flagship) and tier `middle` (slot 2).
export const PROPIMAGES_PROFILE = 'release';
export const PROPIMAGES_TIERS = Object.freeze(['large', 'middle']);
export const SLOTS = Object.freeze([1, 2]);
export const MAX_IMAGES = 8; // propimage_runspec.MAX_IMAGES
export const MAX_APPS = 32; // propimage_runspec.check_image
export const LANES = Object.freeze(['gate', 'discovery']);
const SHA256 = /^[0-9a-f]{64}$/;
const LABEL = /^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$/; // propimage_runspec.LABEL_RE
const APP = /^[a-z0-9][a-z0-9_]{0,31}$/; // propimage_runspec.APP_RE
const REF = /^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/;
const KEY = /^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$/;
const ID = /^[a-z][a-z0-9]{0,15}$/;
const ROLES = ['image', 'gop', 'linux_image'];
const MAX_LINUX_IMAGES = 4;
// The row ids of scripts/bench/linux/matlab_rows.py (ROW_ORDER); test/propimages.test.js compares this list with the box's own table.
// The lane of every row of an image on a family it has not run on.
const LANE_UNMEASURED = 'discovery';
export const MATLAB_ROWS = Object.freeze([
  'gpu_info',
  'gpu_matmul',
  'gpu_fft',
  'gpu_solve',
  'dl_train',
  'parpool_gpu',
  'cuda_kernel',
  'mem_oversub',
]);
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
// A driver version as the release matrix writes it: two to four numeric fields (590.48.01).
const DOTTED_VERSION = /^[0-9]+(\.[0-9]+){1,3}$/;
// The guest binary of a Linux image: the box rows' own rule (scripts/bench/linux/matlab_rows.py BIN_RE, and the instruction schema's `matlab.bin`). The `$` is
// the end of the string here (no multiline flag), so a trailing newline does not match.
const GUEST_BIN = /^[A-Za-z0-9_./-]{1,120}$/;
const pinnedLinuxInstaller = (matrix, version) =>
  (matrix.driver_installers ?? []).find(
    (p) => p.os === 'linux' && p.version === version && p.status === 'PINNED',
  );
// Numeric field-by-field comparison of two dotted versions (a missing field is 0): negative when a is older than b. Never a text compare
// ('1000.2.3' is newer than '590.48.01').
export function compareDottedVersions(a, b) {
  const x = String(a).split('.').map(Number);
  const y = String(b).split('.').map(Number);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d;
  }
  return 0;
}
const only = (v, allowed) => isObject(v) && Object.keys(v).every((k) => allowed.includes(k));

// Every problem of a configuration, as upper-case tokens with an optional `:detail`; an empty list is a good configuration.
// `registry` (optional) = { label: [known app ids] }: the apps the runner has a definition for. When given, an application
// the runner does not know is UNKNOWN_APP (a strict run refuses it by name before any boot; this refuses it earlier).
export function propimagesConfigProblems(
  value = PROPIMAGES,
  { registry, releaseMatrix = RELEASE_MATRIX } = {},
) {
  const problems = [];
  const bad = (reason, ok = false) => {
    if (!ok) problems.push(reason);
    return ok;
  };
  if (
    !bad(
      'SCHEMA',
      only(value, [
        'schema',
        'status',
        'strict',
        'swap',
        'unmeasured_note',
        'objects',
        'images',
        'linux_images',
      ]),
    )
  )
    return problems;
  // the release-level exchange of the two slots (default no swap); the per-release rotation is applied on top of it
  bad('SWAP', typeof value.swap === 'boolean');
  bad(
    'UNMEASURED_NOTE',
    typeof value.unmeasured_note === 'string' && value.unmeasured_note.length > 0,
  );
  bad('SCHEMA', value.schema === 'kayfabe.worker-propimages/1');
  // the Worker/CI run is strict, always: a configuration that is not strict is refused, never "made strict" quietly
  bad('NOT_STRICT', value.strict === true);
  const objects = new Map();
  if (!bad('OBJECTS', Array.isArray(value.objects) && value.objects.length > 0)) return problems;
  for (const o of value.objects) {
    if (!bad('OBJECT_SCHEMA', only(o, ['id', 'role', 'ref', 'object', 'size', 'sha256']))) continue;
    const id = String(o.id);
    bad('OBJECT_ID:' + id.slice(0, 16), typeof o.id === 'string' && ID.test(o.id));
    bad('DUPLICATE_OBJECT:' + id.slice(0, 16), !objects.has(o.id));
    objects.set(o.id, o);
    bad('OBJECT_ROLE:' + id, ROLES.includes(o.role));
    bad('OBJECT_REF:' + id, typeof o.ref === 'string' && REF.test(o.ref));
    bad(
      'OBJECT_KEY:' + id,
      typeof o.object === 'string' && KEY.test(o.object) && !o.object.split('/').includes('..'),
    );
    // an object with no size or no sha256 pin is never served: the box refuses an unpinned object by name, and so does this
    bad('UNPINNED:' + id, typeof o.sha256 === 'string' && SHA256.test(o.sha256));
    bad('SIZE:' + id, Number.isSafeInteger(o.size) && o.size > 0);
  }
  const refs = value.objects.map((o) => o?.ref);
  bad('DUPLICATE_REF', new Set(refs).size === refs.length);
  bad('GOP', value.objects.filter((o) => o?.role === 'gop').length === 1);
  const images = value.images;
  if (!bad('IMAGES', Array.isArray(images) && images.length > 0 && images.length <= MAX_IMAGES))
    return problems;
  const labels = new Set();
  const slots = new Set();
  const families = Object.keys(DEFAULTS.families);
  for (const img of images) {
    if (
      !bad(
        'IMAGE_SCHEMA',
        only(img, [
          'label',
          'object',
          'slot',
          'measured_families',
          'measured_basis',
          'apps',
          'excluded',
        ]),
      )
    )
      continue;
    const label = String(img.label);
    bad('LABEL:' + label.slice(0, 32), typeof img.label === 'string' && LABEL.test(img.label));
    bad('DUPLICATE_LABEL:' + label.slice(0, 32), !labels.has(img.label));
    labels.add(img.label);
    bad('UNKNOWN_OBJECT:' + label, objects.get(img.object)?.role === 'image');
    // the box slot of the family's two boxes this image runs on (1 = the flagship die); one image per slot
    bad('SLOT:' + label, SLOTS.includes(img.slot) && !slots.has(img.slot));
    slots.add(img.slot);
    // the families the image has run on: a lane is trusted there. Any other family runs every application in discovery.
    bad(
      'MEASURED_FAMILIES:' + label,
      Array.isArray(img.measured_families) &&
        new Set(img.measured_families).size === img.measured_families.length &&
        img.measured_families.every((f) => families.includes(f)),
    );
    const apps = img.apps;
    if (!bad('NO_APPS:' + label, isObject(apps) && Object.keys(apps).length > 0)) continue;
    bad('TOO_MANY_APPS:' + label, Object.keys(apps).length <= MAX_APPS);
    for (const [id, row] of Object.entries(apps)) {
      // an all-digit id would be reordered by an object key sort: refused so the canonical form stays the runner's
      bad('APP_ID:' + label + ':' + id.slice(0, 32), APP.test(id) && !/^[0-9]+$/.test(id));
      bad(
        'LANE:' + label + ':' + id.slice(0, 32),
        only(row, ['lane', 'basis']) && LANES.includes(row.lane),
      );
      if (registry !== undefined)
        bad(
          'UNKNOWN_APP:' + label + ':' + id.slice(0, 32),
          registry[img.label]?.includes(id) === true,
        );
    }
    if (img.excluded !== undefined) {
      const ex = img.excluded;
      bad(
        'EXCLUDED:' + label,
        isObject(ex) && Object.keys(ex).every((id) => APP.test(id) && !(id in apps)),
      );
    }
  }
  if (value.linux_images !== undefined)
    linuxImageProblems(value, objects, labels, families, bad, releaseMatrix);
  return problems;
}

// The Linux images (the MATLAB image): validated beside the Windows images but outside the slot rule. An entry names its private
// object (role `linux_image`), says that no per-family list is kept (`families: 'generic'`), how its boots are isolated, how it is
// scheduled, and its rows with their lanes. `measured_families` (optional) is the families a lane is trusted on; elsewhere every row
// is emitted in the discovery lane, exactly as for the Windows images.
function linuxImageProblems(value, objects, labels, families, bad, releaseMatrix) {
  const list = value.linux_images;
  if (
    !bad('LINUX_IMAGES', Array.isArray(list) && list.length > 0 && list.length <= MAX_LINUX_IMAGES)
  )
    return;
  for (const li of list) {
    if (
      !bad(
        'LINUX_IMAGE_SCHEMA',
        only(li, [
          'label',
          'object',
          'families',
          'isolation',
          'scheduling',
          'min_guest_driver',
          'min_guest_driver_basis',
          'bin',
          'bin_basis',
          'measured_families',
          'rows',
        ]),
      )
    )
      continue;
    const label = String(li.label);
    bad('LABEL:' + label.slice(0, 32), typeof li.label === 'string' && LABEL.test(li.label));
    bad('DUPLICATE_LABEL:' + label.slice(0, 32), !labels.has(li.label));
    labels.add(li.label);
    bad('LINUX_OBJECT:' + label, objects.get(li.object)?.role === 'linux_image');
    bad('LINUX_FAMILIES:' + label, li.families === 'generic');
    bad('LINUX_ISOLATION:' + label, ['row', 'shared'].includes(li.isolation));
    bad('LINUX_SCHEDULING:' + label, typeof li.scheduling === 'string' && li.scheduling.length > 0);
    // `min_guest_driver` (optional): the oldest guest driver the image's application accepts. It is a dotted version that is a PINNED Linux
    // installer of the release matrix (the Worker never names a driver the box cannot fetch by pin), and it carries the basis of the claim.
    // The basis is allowed only beside the minimum: a note about nothing is a stray key.
    if (li.min_guest_driver !== undefined)
      bad(
        'MIN_GUEST_DRIVER:' + label,
        typeof li.min_guest_driver === 'string' &&
          DOTTED_VERSION.test(li.min_guest_driver) &&
          pinnedLinuxInstaller(releaseMatrix, li.min_guest_driver) !== undefined,
      );
    if (li.min_guest_driver !== undefined || li.min_guest_driver_basis !== undefined)
      bad(
        'MIN_GUEST_DRIVER_BASIS:' + label,
        li.min_guest_driver !== undefined &&
          typeof li.min_guest_driver_basis === 'string' &&
          li.min_guest_driver_basis.length > 0,
      );
    // `bin` (optional, 2026-11-18): where the image keeps the guest's matlab binary. It is a property of the IMAGE (measured: `matlab` is not on the guest's ssh
    // PATH in the MATLAB Linux image), so it is configuration, never a name in code; the Worker copies it into the instruction's `matlab.bin`. The basis (how
    // it was found) is allowed only beside it, like the minimum driver's.
    if (li.bin !== undefined)
      bad('BIN:' + label, typeof li.bin === 'string' && GUEST_BIN.test(li.bin));
    if (li.bin !== undefined || li.bin_basis !== undefined)
      bad(
        'BIN_BASIS:' + label,
        li.bin !== undefined && typeof li.bin_basis === 'string' && li.bin_basis.length > 0,
      );
    if (li.measured_families !== undefined)
      bad(
        'MEASURED_FAMILIES:' + label,
        Array.isArray(li.measured_families) &&
          new Set(li.measured_families).size === li.measured_families.length &&
          li.measured_families.every((f) => families.includes(f)),
      );
    const rows = li.rows;
    if (!bad('NO_ROWS:' + label, isObject(rows) && Object.keys(rows).length > 0)) continue;
    for (const [id, row] of Object.entries(rows)) {
      bad('ROW_ID:' + label + ':' + id.slice(0, 32), MATLAB_ROWS.includes(id));
      bad(
        'LANE:' + label + ':' + id.slice(0, 32),
        only(row, ['lane', 'basis']) && LANES.includes(row?.lane),
      );
    }
  }
}
export function checkPropimagesConfig(value = PROPIMAGES, options) {
  const problems = propimagesConfigProblems(value, options);
  if (problems.length) throw Error('propimages config: ' + problems[0]);
  return value;
}

// Which of the family's two boxes a die is: its 1-based place among the release matrix rows of its family, the order the Worker
// plans them in (src/release.js planRelease: the flagship first, which runs leg tier `large`; the second die runs `middle`).
export function slotOfDie(die, { cfg = {}, releaseMatrix = RELEASE_MATRIX } = {}) {
  const family = familyOf(die, cfg);
  const rows = releaseMatrix.dies.filter((d) => d.die_family === family);
  const at = rows.findIndex((d) => d.die === die);
  if (at < 0 || at >= SLOTS.length) throw Error('propimages slot:' + String(die).slice(0, 16));
  return at + 1;
}

// Who runs which image (the plan, with the reasons the Worker prints in the leg's log). The rule, in two sentences: image slot 1
// runs on the family's first box and image slot 2 on its second, exchanged when `config.swap` XOR (releaseSeq is odd), so
// consecutive releases rotate which die gets which image while one release (one releaseSeq, hence a retry on the same sha) always
// gets the same assignment; a family with one die available (`familyDies` 1, also the one-die rehearsal) runs BOTH images on that
// box and is not rotated. `releaseSeq` is a request field (a non-negative integer; the Worker takes it from the admin start of the
// release), never state kept here. Lanes: configured for a family the image has run on, discovery for every other family.
export function propimagePlan(
  { profile, tierName, die, familyDies, releaseSeq, images } = {},
  { config: value = PROPIMAGES, cfg = {}, releaseMatrix = RELEASE_MATRIX } = {},
) {
  checkPropimagesConfig(value);
  if (profile !== PROPIMAGES_PROFILE) throw Error('propimages release only');
  if (!PROPIMAGES_TIERS.includes(tierName)) throw Error('propimages tier');
  const family = familyOf(die, cfg);
  if (family === 'unknown') throw Error('propimages die');
  if (!Object.hasOwn(cfg.releaseDies ?? DEFAULTS.releaseDies, family))
    throw Error('propimages family:' + family);
  const known = new Map(value.images.map((img) => [img.label, img]));
  const lines = [];
  let chosen;
  let assignment;
  if (images !== undefined) {
    // an explicit selection (a manual or standalone run); it bypasses the slot rule and says so
    if (!Array.isArray(images) || !images.length || new Set(images).size !== images.length)
      throw Error('propimages images');
    chosen = images.map((label) => {
      const img = known.get(label);
      if (img === undefined) throw Error('propimages unknown image:' + String(label).slice(0, 32));
      return img;
    });
    assignment = { mode: 'explicit' };
    lines.push(
      'images named by the request (' + images.join(', ') + '), the slot rule is not applied',
    );
  } else {
    const count = familyDies ?? DEFAULTS.releaseDiesPerFamily;
    if (!SLOTS.includes(count)) throw Error('propimages dies');
    if (count === 1) {
      chosen = value.images;
      assignment = { mode: 'single-die', slot: null, swap: false };
      lines.push(
        'only one die of family ' +
          family +
          ' is available: this box runs BOTH images (' +
          chosen.map((i) => i.label).join(', ') +
          '), no rotation',
      );
    } else {
      if (!Number.isSafeInteger(releaseSeq) || releaseSeq < 0) throw Error('propimages releaseSeq');
      const slot = slotOfDie(die, { cfg, releaseMatrix });
      const rotation = releaseSeq % 2 === 1;
      const swap = value.swap !== rotation;
      // an image's box slot: its own slot, exchanged when the effective swap is on
      const boxSlot = (img) => (swap ? SLOTS.length + 1 - img.slot : img.slot);
      chosen = value.images.filter((img) => boxSlot(img) === slot);
      assignment = { mode: 'split', slot, swap, config_swap: value.swap, releaseSeq, rotation };
      lines.push(
        'die ' +
          die +
          ' (' +
          family +
          ') is box slot ' +
          slot +
          ' of ' +
          count +
          '; release ' +
          releaseSeq +
          ' is ' +
          (rotation ? 'odd' : 'even') +
          ' and config swap is ' +
          (value.swap ? 'on' : 'off') +
          ', so the effective swap is ' +
          (swap ? 'on' : 'off'),
      );
      for (const img of value.images)
        lines.push(
          '  image ' +
            img.label +
            ': configured slot ' +
            img.slot +
            ', box slot ' +
            boxSlot(img) +
            (boxSlot(img) === slot ? ' -> RUNS HERE' : ' (runs on the other box)'),
        );
      if (!chosen.length) throw Error('propimages no image for slot:' + slot);
    }
  }
  const objects = new Map(value.objects.map((o) => [o.id, o]));
  const part = {
    propimages: chosen.map((img) => {
      const object = objects.get(img.object);
      const measured = img.measured_families.includes(family);
      lines.push(
        '  ' +
          img.label +
          ' on ' +
          family +
          ': ' +
          (measured
            ? 'lanes as configured'
            : value.unmeasured_note + ': every application in the discovery lane'),
      );
      return {
        label: img.label,
        kind: 'prebuilt',
        disk: { from: 'pin', ref: object.ref, sha256: object.sha256 },
        apps: Object.fromEntries(
          Object.entries(img.apps).map(([id, row]) => [
            id,
            { mode: 'present', lane: measured ? row.lane : 'discovery' },
          ]),
        ),
      };
    }),
    // The Worker/CI run is strict. There is no option to say otherwise.
    propimages_strict: true,
  };
  return { part, assignment: { ...assignment, family, images: chosen.map((i) => i.label) }, lines };
}
// The `matlab` part of the instruction for ONE release leg (the stage `matlab` of scripts/ci/vast_payload.py, validated by the box's schema and
// by scripts/ci/matlab_stage.py instruction_refusal). A Linux image needs a KVM box with a GPU; it is NOT part of the Windows A/B slot rotation:
// the part depends on the leg's die only (its pinned guest driver from the release matrix, raised to the image's `min_guest_driver` when older)
// and is emitted for the large-tier leg of a release.
// Always strict: the image is a pin, the rows are the configured ones, and a family the image has not run on gets every row in the discovery lane.
export function matlabPlan(
  { profile, tierName, die, label = 'matlab' } = {},
  { config: value = PROPIMAGES, cfg = {}, releaseMatrix = RELEASE_MATRIX } = {},
) {
  checkPropimagesConfig(value);
  if (profile !== PROPIMAGES_PROFILE) throw Error('matlab release only');
  if (tierName !== 'large') throw Error('matlab tier');
  const family = familyOf(die, cfg);
  if (family === 'unknown') throw Error('matlab die');
  const li = (value.linux_images ?? []).find((i) => i.label === label);
  if (li === undefined) throw Error('matlab unknown image:' + String(label).slice(0, 32));
  const version = releaseMatrix.dies.find((d) => d.die === die)?.drivers?.linux_guest_driver;
  const diePin = pinnedLinuxInstaller(releaseMatrix, version);
  if (diePin === undefined) throw Error('matlab driver:' + String(die).slice(0, 16));
  // The image's application refuses a guest driver older than its CUDA runtime (propimages.json `min_guest_driver`, measured on one die): a die
  // whose default is older gets exactly the pinned minimum (never a newer driver than needed), any other die keeps its own default. A
  // minimum that is not a pinned Linux installer of THIS matrix refuses the plan by name, whether or not this die needs it.
  let pin = diePin;
  let why = '';
  if (li.min_guest_driver !== undefined) {
    const minPin = pinnedLinuxInstaller(releaseMatrix, li.min_guest_driver);
    if (minPin === undefined)
      throw Error('matlab min driver:' + String(li.min_guest_driver).slice(0, 16));
    if (compareDottedVersions(diePin.version, minPin.version) < 0) {
      pin = minPin;
      why =
        ' (the image needs at least ' +
        minPin.version +
        "; the die's default " +
        diePin.version +
        ' is older)';
    }
  }
  const object = value.objects.find((o) => o.id === li.object);
  const measured = (li.measured_families ?? []).includes(family);
  const part = {
    matlab: {
      label: li.label,
      image: { from: 'pin', ref: object.ref, sha256: object.sha256 },
      driver: { version: pin.version },
      rows: Object.fromEntries(
        Object.entries(li.rows).map(([id, row]) => [
          id,
          { lane: measured ? row.lane : LANE_UNMEASURED },
        ]),
      ),
      isolation: li.isolation,
      // the image's own binary (propimages.json `bin`), only when it is configured: absent = the box runs the name `matlab`
      ...(li.bin !== undefined ? { bin: li.bin } : {}),
    },
  };
  const lines = [
    li.label +
      ' on ' +
      die +
      ' (' +
      family +
      '): guest driver ' +
      pin.version +
      why +
      ', ' +
      (measured
        ? 'lanes as configured'
        : value.unmeasured_note + ': every row in the discovery lane') +
      '; ' +
      li.scheduling,
  ];
  return { part, lines };
}

// The `propimages` / `propimages_strict` part of the instruction for ONE leg (see propimagePlan for the rule and the refusals).
export const propimagesForLeg = (leg, options) => propimagePlan(leg, options).part;
// Which images a leg takes, or [] when it takes none (the planner asks this before it asks for the instruction).
export function propimageLabelsFor(leg, options) {
  try {
    return propimagePlan(leg, options).assignment.images;
  } catch {
    return [];
  }
}

// The instruction fields a box derives from a Worker environment (the adapter vast_payload.instruction_from_env: KF_* in,
// the typed instruction out), for the environments tierEnvironment sends. A leg with driver-matrix fields is not expressible:
// the instruction has no driver fields and the payload refuses a matrix leg while PAYLOAD_RELEASE_MATRIX_EXECUTOR is 0.
export function instructionFromEnvironment(env) {
  return {
    tier: env.KF_TIER ?? 'unknown',
    windows: env.KF_MERGE_WINDOWS !== '0',
    windows_runs: env.KF_WINDOWS_RUNS === '2' ? 2 : 1,
    stages: env.KF_STAGES ? env.KF_STAGES.split(':') : [],
    spec: env.KF_SPEC ?? null,
    winapps: env.KF_WINAPPS ?? null,
    linux_apps: env.KF_LINUX_APPS === '1' ? 'middle' : null,
    rehearsal: env.KF_RELEASE_REHEARSAL === '1',
  };
}

// The whole instruction of a leg. The base is exactly what tierEnvironment sends for the same options. The `propimages` part is
// added ONLY when the caller passes `propimages: true` (default off, so no live leg changes), for a release-profile leg, with the
// leg's `die` and the release's `releaseSeq`; `familyDies` (1 or 2, default the release's two) says how many dies of the family
// this release has. A one-die rehearsal is a single-die leg. Returns the instruction; `buildLeg` also returns the plan text.
export function buildLeg(
  profile,
  {
    propimages = false,
    propimageImages,
    matlab = false,
    die,
    releaseSeq,
    familyDies,
    ...tierOptions
  } = {},
  { config: value = PROPIMAGES, cfg = {}, releaseMatrix = RELEASE_MATRIX } = {},
) {
  if (tierOptions.releaseMatrix !== undefined) throw Error('instruction release matrix');
  const instruction = {
    schema: INSTRUCTION_SCHEMA,
    ...instructionFromEnvironment(tierEnvironment(profile, tierOptions)),
  };
  let plan;
  if (propimages !== false) {
    if (propimages !== true) throw Error('propimages flag');
    plan = propimagePlan(
      {
        profile,
        tierName: tierOptions.tierName,
        die,
        releaseSeq,
        images: propimageImages,
        familyDies: familyDies ?? (tierOptions.rehearsal === true ? 1 : undefined),
      },
      { config: value, cfg, releaseMatrix },
    );
    Object.assign(instruction, plan.part);
  } else if (propimageImages !== undefined) throw Error('propimages images without the flag');
  if (matlab !== false) {
    if (matlab !== true) throw Error('matlab flag');
    const m = matlabPlan(
      { profile, tierName: tierOptions.tierName, die },
      { config: value, cfg, releaseMatrix },
    );
    Object.assign(instruction, m.part);
    plan =
      plan === undefined ? { lines: m.lines } : { ...plan, lines: [...plan.lines, ...m.lines] };
  }
  return { instruction, plan };
}
export const buildInstruction = (profile, options, config) =>
  buildLeg(profile, options, config).instruction;

// The bytes scripts/ci/payload_instruction.py writes (json.dumps(indent=2, sort_keys=True) + newline), so the Worker's
// instruction and the box's emitted one can be compared byte for byte (and hashed to the same value).
const sorted = (v) =>
  Array.isArray(v)
    ? v.map(sorted)
    : isObject(v)
      ? Object.fromEntries(
          Object.keys(v)
            .sort()
            .map((k) => [k, sorted(v[k])]),
        )
      : v;
export const canonicalInstruction = (doc) => JSON.stringify(sorted(doc), null, 2) + '\n';
