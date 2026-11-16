// The Worker's knowledge of the proprietary Windows images (docs/design/V3_WORKER_PROPIMAGES.md).
// STATUS: DESIGN-ONLY, 2026-11-16. Nothing in src/index.js imports this file: it is pure, tested, and not wired into any
// rental. The deployed Worker is unchanged. Owner decision 2026-11-16: the Worker knows image A and image B from
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
const ROLES = ['image', 'gop'];
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const only = (v, allowed) => isObject(v) && Object.keys(v).every((k) => allowed.includes(k));

// Every problem of a configuration, as upper-case tokens with an optional `:detail`; an empty list is a good configuration.
// `registry` (optional) = { label: [known app ids] }: the apps the runner has a definition for. When given, an application
// the runner does not know is UNKNOWN_APP (a strict run refuses it by name before any boot; this refuses it earlier).
export function propimagesConfigProblems(value = PROPIMAGES, { registry } = {}) {
  const problems = [];
  const bad = (reason, ok = false) => {
    if (!ok) problems.push(reason);
    return ok;
  };
  if (
    !bad(
      'SCHEMA',
      only(value, ['schema', 'status', 'strict', 'swap', 'unmeasured_note', 'objects', 'images']),
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
  return problems;
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
  { propimages = false, propimageImages, die, releaseSeq, familyDies, ...tierOptions } = {},
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
