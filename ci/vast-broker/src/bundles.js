// The propimage bundle table: schema validation and the application resolver (a port of scripts/ci/propimage_config.py).
// STATUS: LIVE as a pure, tested module, 2026-11-19. Nothing in src/index.js imports it; the offline planner uses it only to check its configured
// images against the table. Design: docs/design/V3_PROPIMAGE_CONFIG.md. The reference is the Python module: test/fixtures/bundles/parity-cases.json
// is asserted by both (scripts/ci/test_propimage_config.py and test/bundles.test.js), reasons included, so the two cannot drift.
// Not ported: duplicate object keys in the JSON text (JSON.parse cannot see them; the Python loader and its test do).
import bundlesFile from '../propimage-bundles.json' with { type: 'json' };

export const BUNDLES = bundlesFile;
export const SCHEMA = 'kayfabe.propimage-bundles/1';
export const OSES = Object.freeze(['windows', 'linux']);
export const FAMILIES = Object.freeze(['turing', 'ampere', 'ada', 'blackwell']);
export const ISOLATIONS = Object.freeze(['row', 'shared']);
export const MAX_BUNDLES = 16;
export const MAX_APPS = 64;
export const MAX_SIZE = 2 ** 42;
const BUNDLE_ID = /^[a-z0-9][a-z0-9_-]{0,31}$/;
const APP = /^[a-z0-9][a-z0-9_]{0,31}$/;
const KEY = /^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$/;
const REF = /^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$/;
const PATH = /^[A-Za-z0-9][A-Za-z0-9._/-]{0,199}$/;
const WORD = /^[A-Za-z0-9._:=/-]{1,64}$/;
const HEX64 = /^[0-9a-f]{64}$/;
const TEXT = /^[\x20-\x7e]{1,300}$/;
const SECRETISH = /@|[A-Za-z0-9]{24,}/;
const BUNDLE_KEYS = [
  'os',
  'r2',
  'size_bytes',
  'apps',
  'sha256',
  'ref',
  'licence_note',
  'basis',
  'needs_gpu_families',
  'run',
];
const BUNDLE_REQUIRED = ['os', 'r2', 'size_bytes', 'apps'];
const WINDOWS_RUN_KEYS = [
  'rows_file',
  'pins_file',
  'rows',
  'solo_rows',
  'data_iso',
  'guest_ram_mb',
  'audio',
  'harness_args',
];
const LINUX_RUN_KEYS = ['rows_file', 'pins_file', 'isolation'];
const ARG_KEYS = ['before_apps', 'after_apps', 'per_row'];

export class ConfigError extends Error {
  constructor(reason) {
    super(reason);
    this.reason = reason;
  }
}
const refuse = (condition, reason) => {
  if (!condition) throw new ConfigError(reason);
};
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const isInt = (v) => Number.isSafeInteger(v);
const has = (o, k) => Object.hasOwn(o, k);
const subset = (o, allowed) => Object.keys(o).every((k) => allowed.includes(k));
const isString = (v) => typeof v === 'string';

function uniqueStrings(value, regex, reason, limit = MAX_APPS) {
  refuse(Array.isArray(value) && value.length > 0 && value.length <= limit, reason);
  refuse(
    value.every((x) => isString(x) && regex.test(x)),
    reason,
  );
  refuse(new Set(value).size === value.length, reason);
}
function text(value, name, bid) {
  refuse(isString(value) && TEXT.test(value), `${name}:${bid}`);
  refuse(!SECRETISH.test(value), `${name}_SECRETISH:${bid}`);
}
function relativePath(value, reason) {
  refuse(isString(value) && PATH.test(value), reason);
  refuse(!value.split('/').includes('..') && !value.includes('//') && !value.endsWith('/'), reason);
}
function words(value, bid, where) {
  refuse(
    Array.isArray(value) && value.length <= 16 && value.every((w) => isString(w) && WORD.test(w)),
    `ARGS:${bid}:${where}`,
  );
}

function checkWindowsRun(bid, run) {
  refuse(subset(run, WINDOWS_RUN_KEYS), 'RUN_KEY:' + bid);
  refuse(has(run, 'rows_file') && has(run, 'pins_file'), 'RUN_MISSING:' + bid);
  relativePath(run.rows_file, 'RUN_ROWS_FILE:' + bid);
  relativePath(run.pins_file, 'RUN_PINS_FILE:' + bid);
  const rows = has(run, 'rows') ? run.rows : null;
  if (rows !== null) uniqueStrings(rows, APP, 'RUN_ROWS:' + bid, 128);
  if (has(run, 'solo_rows')) {
    uniqueStrings(run.solo_rows, APP, 'RUN_SOLO_ROWS:' + bid, 128);
    if (rows !== null)
      refuse(
        run.solo_rows.every((r) => rows.includes(r)),
        'RUN_SOLO_ROWS_UNKNOWN:' + bid,
      );
  }
  if (has(run, 'data_iso')) refuse(typeof run.data_iso === 'boolean', 'RUN_DATA_ISO:' + bid);
  if (has(run, 'guest_ram_mb')) {
    const v = run.guest_ram_mb;
    refuse(
      isInt(v) && (v === 0 || (v >= 1024 && v <= 2 ** 20 && v % 1024 === 0)),
      'RUN_GUEST_RAM:' + bid,
    );
  }
  if (has(run, 'audio')) refuse(typeof run.audio === 'boolean', 'RUN_AUDIO:' + bid);
  if (has(run, 'harness_args')) {
    const args = run.harness_args;
    refuse(isObject(args) && subset(args, ARG_KEYS), 'ARGS_KEY:' + bid);
    for (const where of ['before_apps', 'after_apps'])
      if (has(args, where)) words(args[where], bid, where);
    const perRow = has(args, 'per_row') ? args.per_row : {};
    refuse(isObject(perRow), 'ARGS_PER_ROW:' + bid);
    for (const [row, w] of Object.entries(perRow)) {
      refuse(APP.test(row), 'ARGS_PER_ROW_NAME:' + bid);
      if (rows !== null) refuse(rows.includes(row), `ARGS_PER_ROW_UNKNOWN:${bid}:${row}`);
      words(w, bid, 'per_row:' + row);
    }
    const flat = ['before_apps', 'after_apps'].flatMap((k) => (has(args, k) ? args[k] : []));
    refuse(
      !flat.includes('--data-iso') && !Object.values(perRow).some((w) => w.includes('--data-iso')),
      'ARGS_DATA_ISO_SET_BY_FIELD:' + bid,
    );
  }
}
function checkLinuxRun(bid, run) {
  refuse(subset(run, LINUX_RUN_KEYS), 'RUN_KEY:' + bid);
  refuse(
    LINUX_RUN_KEYS.every((k) => has(run, k)),
    'RUN_MISSING:' + bid,
  );
  relativePath(run.rows_file, 'RUN_ROWS_FILE:' + bid);
  relativePath(run.pins_file, 'RUN_PINS_FILE:' + bid);
  refuse(ISOLATIONS.includes(run.isolation), 'RUN_ISOLATION:' + bid);
}
function checkBundle(bid, b) {
  refuse(BUNDLE_ID.test(bid), 'BUNDLE_ID:' + bid.slice(0, 32));
  refuse(isObject(b), 'BUNDLE_SHAPE:' + bid);
  refuse(subset(b, BUNDLE_KEYS), 'BUNDLE_KEY:' + bid);
  refuse(
    BUNDLE_REQUIRED.every((k) => has(b, k)),
    'BUNDLE_MISSING:' + bid,
  );
  refuse(OSES.includes(b.os), 'BUNDLE_OS:' + bid);
  refuse(isString(b.r2) && KEY.test(b.r2), 'BUNDLE_R2:' + bid);
  refuse(
    !b.r2.split('/').includes('..') && !b.r2.includes('//') && !b.r2.endsWith('/'),
    'BUNDLE_R2:' + bid,
  );
  refuse(isInt(b.size_bytes) && b.size_bytes > 0 && b.size_bytes < MAX_SIZE, 'BUNDLE_SIZE:' + bid);
  uniqueStrings(b.apps, APP, 'BUNDLE_APPS:' + bid);
  if (has(b, 'sha256')) refuse(isString(b.sha256) && HEX64.test(b.sha256), 'BUNDLE_SHA256:' + bid);
  if (has(b, 'ref')) refuse(isString(b.ref) && REF.test(b.ref), 'BUNDLE_REF:' + bid);
  for (const [key, name] of [
    ['licence_note', 'BUNDLE_LICENCE_NOTE'],
    ['basis', 'BUNDLE_BASIS'],
  ])
    if (has(b, key)) text(b[key], name, bid);
  if (has(b, 'needs_gpu_families')) {
    const f = b.needs_gpu_families;
    refuse(
      Array.isArray(f) &&
        f.length > 0 &&
        f.every((x) => FAMILIES.includes(x)) &&
        new Set(f).size === f.length,
      'BUNDLE_FAMILIES:' + bid,
    );
  }
  if (has(b, 'run')) {
    refuse(isObject(b.run), 'RUN_SHAPE:' + bid);
    if (b.os === 'windows') checkWindowsRun(bid, b.run);
    else checkLinuxRun(bid, b.run);
  }
}

// The document itself, or a ConfigError whose `reason` is the Python module's reason string.
export function validate(doc) {
  refuse(isObject(doc) && doc.schema === SCHEMA, 'SCHEMA');
  refuse(subset(doc, ['schema', 'status', 'bundles']) && has(doc, 'bundles'), 'TOP_KEY');
  if (has(doc, 'status')) refuse(isString(doc.status) && doc.status.length <= 4000, 'STATUS');
  const bundles = doc.bundles;
  refuse(
    isObject(bundles) &&
      Object.keys(bundles).length > 0 &&
      Object.keys(bundles).length <= MAX_BUNDLES,
    'BUNDLES',
  );
  for (const [bid, b] of Object.entries(bundles)) checkBundle(bid, b);
  const keys = Object.values(bundles).map((b) => b.r2);
  refuse(new Set(keys).size === keys.length, 'DUPLICATE_R2');
  return doc;
}

const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);
function* combinations(items, count, from = 0, head = []) {
  if (head.length === count) {
    yield head;
    return;
  }
  for (let i = from; i < items.length; i++)
    yield* combinations(items, count, i + 1, [...head, items[i]]);
}
// The bundle ids to fetch so that every requested application is on one of them: fewest bytes, then fewest bundles, then the smallest id
// list; sorted by id. `os` restricts the platform; without it an application on both platforms is AMBIGUOUS_APP.
export function bundlesFor(config, apps, os = null) {
  const bundles = validate(config).bundles;
  refuse(os === null || OSES.includes(os), 'BUNDLE_OS:' + String(os).slice(0, 16));
  refuse((Array.isArray(apps) || apps instanceof Set) && [...apps].every(isString), 'APPS_SHAPE');
  const wanted = [...new Set(apps)].sort(cmp);
  for (const name of wanted) refuse(APP.test(name), 'BAD_APP:' + name.slice(0, 32));
  const holders = {};
  for (const name of wanted) {
    const have = Object.keys(bundles)
      .filter((id) => bundles[id].apps.includes(name) && (os === null || bundles[id].os === os))
      .sort(cmp);
    refuse(have.length > 0, 'UNKNOWN_APP:' + name);
    refuse(new Set(have.map((id) => bundles[id].os)).size === 1, 'AMBIGUOUS_APP:' + name);
    holders[name] = have;
  }
  if (wanted.length === 0) return [];
  const relevant = [...new Set(Object.values(holders).flat())].sort(cmp);
  let best = null;
  for (let count = 1; count <= relevant.length; count++)
    for (const pick of combinations(relevant, count)) {
      if (!wanted.every((name) => holders[name].some((id) => pick.includes(id)))) continue;
      const bytes = pick.reduce((sum, id) => sum + bundles[id].size_bytes, 0);
      const better =
        best === null ||
        bytes < best.bytes ||
        (bytes === best.bytes &&
          (count < best.pick.length ||
            (count === best.pick.length && pick.join('\0') < best.pick.join('\0'))));
      if (better) best = { bytes, pick };
    }
  return [...best.pick];
}

// Total bytes of the named bundles; an unknown id is UNKNOWN_BUNDLE.
export function downloadBytes(config, ids) {
  const bundles = validate(config).bundles;
  refuse(
    ids.every((id) => has(bundles, id)),
    'UNKNOWN_BUNDLE',
  );
  return ids.reduce((sum, id) => sum + bundles[id].size_bytes, 0);
}
