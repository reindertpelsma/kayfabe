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

export const PROPIMAGES = config;
export const INSTRUCTION_SCHEMA = 'kayfabe.payload-instruction/1';
// The instruction schema's own rule (payload_instruction.check_semantics): propimages need the large tier.
export const PROPIMAGES_TIER = 'large';
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
  if (!bad('SCHEMA', only(value, ['schema', 'status', 'strict', 'objects', 'images'])))
    return problems;
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
  const families = Object.keys(DEFAULTS.families);
  for (const img of images) {
    if (
      !bad(
        'IMAGE_SCHEMA',
        only(img, ['label', 'object', 'families', 'families_basis', 'apps', 'excluded']),
      )
    )
      continue;
    const label = String(img.label);
    bad('LABEL:' + label.slice(0, 32), typeof img.label === 'string' && LABEL.test(img.label));
    bad('DUPLICATE_LABEL:' + label.slice(0, 32), !labels.has(img.label));
    labels.add(img.label);
    bad('UNKNOWN_OBJECT:' + label, objects.get(img.object)?.role === 'image');
    bad(
      'FAMILIES:' + label,
      Array.isArray(img.families) &&
        img.families.length > 0 &&
        new Set(img.families).size === img.families.length &&
        img.families.every((f) => families.includes(f)),
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

// Which configured images a leg of this tier and die takes. Never throws for a leg that simply has none (a middle leg, a
// family the images have not been measured on): the planner asks this before it asks for the instruction.
export function propimageLabelsFor(
  { tierName, die } = {},
  { config: value = PROPIMAGES, cfg = {} } = {},
) {
  if (tierName !== PROPIMAGES_TIER) return [];
  const family = familyOf(die, cfg);
  return value.images.filter((img) => img.families.includes(family)).map((img) => img.label);
}

// The `propimages` part of the instruction for ONE leg, or a refusal by name. The leg must ASK (this function is only called
// for a leg that asked); `images` optionally names the images by label (any other label is refused by name).
export function propimagesForLeg(
  { tierName, die, images } = {},
  { config: value = PROPIMAGES, cfg = {} } = {},
) {
  checkPropimagesConfig(value);
  if (tierName !== PROPIMAGES_TIER) throw Error('propimages tier');
  const family = familyOf(die, cfg);
  if (family === 'unknown') throw Error('propimages die');
  const known = new Map(value.images.map((img) => [img.label, img]));
  let chosen;
  if (images === undefined) {
    chosen = value.images.filter((img) => img.families.includes(family));
    if (!chosen.length) throw Error('propimages family:' + family);
  } else {
    if (!Array.isArray(images) || !images.length || new Set(images).size !== images.length)
      throw Error('propimages images');
    chosen = images.map((label) => {
      const img = known.get(label);
      if (img === undefined) throw Error('propimages unknown image:' + String(label).slice(0, 32));
      if (!img.families.includes(family)) throw Error('propimages family:' + label + ':' + family);
      return img;
    });
  }
  const objects = new Map(value.objects.map((o) => [o.id, o]));
  return {
    propimages: chosen.map((img) => {
      const object = objects.get(img.object);
      return {
        label: img.label,
        kind: 'prebuilt',
        disk: { from: 'pin', ref: object.ref, sha256: object.sha256 },
        apps: Object.fromEntries(
          Object.entries(img.apps).map(([id, row]) => [id, { mode: 'present', lane: row.lane }]),
        ),
      };
    }),
    // The Worker/CI run is strict. There is no option to say otherwise.
    propimages_strict: true,
  };
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
// added ONLY when the caller passes `propimages: true` (default off, so no live leg changes), with the leg's `die`.
export function buildInstruction(
  profile,
  { propimages = false, propimageImages, die, ...tierOptions } = {},
  { config: value = PROPIMAGES, cfg = {} } = {},
) {
  if (tierOptions.releaseMatrix !== undefined) throw Error('instruction release matrix');
  const doc = {
    schema: INSTRUCTION_SCHEMA,
    ...instructionFromEnvironment(tierEnvironment(profile, tierOptions)),
  };
  if (propimages !== false) {
    if (propimages !== true) throw Error('propimages flag');
    Object.assign(
      doc,
      propimagesForLeg(
        { tierName: tierOptions.tierName, die, images: propimageImages },
        { config: value, cfg },
      ),
    );
  } else if (propimageImages !== undefined) throw Error('propimages images without the flag');
  return doc;
}

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
