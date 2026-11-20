// Mutation check for the bundle table port (src/bundles.js) and the planner's use of it (src/app-lane.js, release-plan.mjs), GPU-free.
// Each mutant is applied in a temporary COPY of the tree (mutant-copy.mjs); test/bundles.test.js and test/app-lane.test.js run, and the
// copy is restored before the next mutant. Every mutant must make at least TWO named tests fail. Run with no other test running here:
//   node ci/vast-broker/bundles-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tempCopy } from './mutant-copy.mjs';

const here = tempCopy();
const TESTS = [here + 'test/bundles.test.js', here + 'test/app-lane.test.js'];
const B = 'src/bundles.js',
  A = 'src/app-lane.js',
  R = 'release-plan.mjs';
// [name, [[file, before, after], ...]]
const MUTANTS = [
  [
    'required key optional',
    [
      [
        B,
        "refuse(\n    BUNDLE_REQUIRED.every((k) => has(b, k)),\n    'BUNDLE_MISSING:' + bid,\n  );",
        '',
      ],
    ],
  ],
  [
    'object key may have a double slash',
    [[B, "&& !b.r2.includes('//') && !b.r2.endsWith('/'),", "&& !b.r2.endsWith('/'),"]],
  ],
  [
    'object key may end in a slash',
    [[B, "&& !b.r2.includes('//') && !b.r2.endsWith('/'),", "&& !b.r2.includes('//'),"]],
  ],
  ['unknown family allowed', [[B, 'f.every((x) => FAMILIES.includes(x)) &&', 'true &&']]],
  ['repeated family allowed', [[B, 'new Set(f).size === f.length,', 'true,']]],
  ['empty family list allowed', [[B, 'f.length > 0 &&', 'true &&']]],
  [
    'guest ram unchecked',
    [[B, 'isInt(v) && (v === 0 || (v >= 1024 && v <= 2 ** 20 && v % 1024 === 0)),', 'true,']],
  ],
  ['linux run required optional', [[B, 'LINUX_RUN_KEYS.every((k) => has(run, k)),', 'true,']]],
  [
    'request shape unchecked',
    [
      [
        B,
        "refuse((Array.isArray(apps) || apps instanceof Set) && [...apps].every(isString), 'APPS_SHAPE');",
        '',
      ],
    ],
  ],
  ['unknown bundle allowed in the sum', [[B, 'ids.every((id) => has(bundles, id)),', 'true,']]],
  [
    'apps ignored by the planner',
    [
      [
        A,
        'opts.apps !== undefined\n      ? bundlesFor(BUNDLES, opts.apps)',
        'false\n      ? bundlesFor(BUNDLES, opts.apps)',
      ],
    ],
  ],
  [
    'bundle count tie-break dropped',
    [
      [
        B,
        "(count < best.pick.length ||\n            (count === best.pick.length && pick.join('\\0') < best.pick.join('\\0')))",
        "pick.join('\\0') < best.pick.join('\\0')",
      ],
    ],
  ],
  ['solo rows need not be rows', [[B, 'run.solo_rows.every((r) => rows.includes(r)),', 'true,']]],
  [
    'bundle lookup by prototype',
    [[B, 'const has = (o, k) => Object.hasOwn(o, k);', 'const has = (o, k) => k in o;']],
  ],
  [
    'image bundle test by prototype',
    [[A, 'Object.hasOwn(bundles?.bundles ?? {}, id)', 'id in (bundles?.bundles ?? {})']],
  ],
  // ---- the document
  [
    'schema not checked',
    [[B, "isObject(doc) && doc.schema === SCHEMA, 'SCHEMA'", "isObject(doc), 'SCHEMA'"]],
  ],
  [
    'top-level typo allowed',
    [
      [
        B,
        "refuse(subset(doc, ['schema', 'status', 'bundles']) && has(doc, 'bundles'), 'TOP_KEY')",
        "refuse(has(doc, 'bundles'), 'TOP_KEY')",
      ],
    ],
  ],
  [
    'bundles key optional',
    [
      [
        B,
        "subset(doc, ['schema', 'status', 'bundles']) && has(doc, 'bundles')",
        "subset(doc, ['schema', 'status', 'bundles'])",
      ],
    ],
  ],
  ['status unbounded', [[B, 'doc.status.length <= 4000', 'true']]],
  ['empty table allowed', [[B, 'Object.keys(bundles).length > 0 &&', '']]],
  ['bundle count unbounded', [[B, 'Object.keys(bundles).length <= MAX_BUNDLES', 'true']]],
  [
    'duplicate object key allowed',
    [[B, "refuse(new Set(keys).size === keys.length, 'DUPLICATE_R2')", '']],
  ],
  // ---- a bundle
  [
    'bundle id unchecked',
    [[B, "refuse(BUNDLE_ID.test(bid), 'BUNDLE_ID:'", "refuse(true, 'BUNDLE_ID:'"]],
  ],
  ['bundle shape unchecked', [[B, "refuse(isObject(b), 'BUNDLE_SHAPE:' + bid);", '']]],
  ['bundle typo allowed', [[B, "refuse(subset(b, BUNDLE_KEYS), 'BUNDLE_KEY:' + bid);", '']]],
  ['os unchecked', [[B, "refuse(OSES.includes(b.os), 'BUNDLE_OS:' + bid);", '']]],
  ['object key shape unchecked', [[B, 'isString(b.r2) && KEY.test(b.r2)', 'isString(b.r2)']]],
  [
    'object key may climb',
    [[B, "!b.r2.split('/').includes('..') && !b.r2.includes('//')", "!b.r2.includes('//')"]],
  ],
  [
    'size zero allowed',
    [
      [
        B,
        'b.size_bytes > 0 && b.size_bytes < MAX_SIZE',
        'b.size_bytes >= 0 && b.size_bytes < MAX_SIZE',
      ],
    ],
  ],
  ['size unbounded', [[B, 'b.size_bytes > 0 && b.size_bytes < MAX_SIZE', 'b.size_bytes > 0']]],
  [
    'size may be a float',
    [
      [
        B,
        'const isInt = (v) => Number.isSafeInteger(v);',
        "const isInt = (v) => typeof v === 'number';",
      ],
    ],
  ],
  [
    'apps may be empty',
    [
      [
        B,
        'Array.isArray(value) && value.length > 0 && value.length <= limit',
        'Array.isArray(value) && value.length <= limit',
      ],
    ],
  ],
  ['apps unbounded', [[B, 'value.length > 0 && value.length <= limit', 'value.length > 0']]],
  [
    'app names unchecked',
    [[B, 'value.every((x) => isString(x) && regex.test(x)),\n    reason,', 'true,\n    reason,']],
  ],
  ['app duplicates allowed', [[B, 'refuse(new Set(value).size === value.length, reason);', '']]],
  [
    'sha256 unchecked',
    [[B, "isString(b.sha256) && HEX64.test(b.sha256), 'BUNDLE_SHA256:'", "true, 'BUNDLE_SHA256:'"]],
  ],
  [
    'ref unchecked',
    [[B, "isString(b.ref) && REF.test(b.ref), 'BUNDLE_REF:'", "true, 'BUNDLE_REF:'"]],
  ],
  // ---- notes
  [
    'e-mail not secretish',
    [[B, 'const SECRETISH = /@|[A-Za-z0-9]{24,}/;', 'const SECRETISH = /[A-Za-z0-9]{24,}/;']],
  ],
  [
    'token not secretish',
    [[B, 'const SECRETISH = /@|[A-Za-z0-9]{24,}/;', 'const SECRETISH = /@|[A-Za-z0-9]{300,}/;']],
  ],
  [
    'secret check dropped',
    [[B, 'refuse(!SECRETISH.test(value), `${name}_SECRETISH:${bid}`);', '']],
  ],
  [
    'note shape unchecked',
    [[B, 'refuse(isString(value) && TEXT.test(value), `${name}:${bid}`);', '']],
  ],
  // ---- the windows run
  ['run shape unchecked', [[B, "refuse(isObject(b.run), 'RUN_SHAPE:' + bid);", '']]],
  ['run typo allowed', [[B, "refuse(subset(run, WINDOWS_RUN_KEYS), 'RUN_KEY:' + bid);", '']]],
  [
    'run required optional',
    [[B, "refuse(has(run, 'rows_file') && has(run, 'pins_file'), 'RUN_MISSING:' + bid);", '']],
  ],
  [
    'rows file unchecked',
    [
      [
        B,
        "relativePath(run.rows_file, 'RUN_ROWS_FILE:' + bid);\n  relativePath(run.pins_file, 'RUN_PINS_FILE:' + bid);\n  const rows",
        "relativePath(run.pins_file, 'RUN_PINS_FILE:' + bid);\n  const rows",
      ],
    ],
  ],
  [
    'path may climb',
    [
      [
        B,
        "refuse(!value.split('/').includes('..') && !value.includes('//') && !value.endsWith('/'), reason);",
        "refuse(!value.includes('//') && !value.endsWith('/'), reason);",
      ],
    ],
  ],
  [
    'rows unchecked',
    [[B, "if (rows !== null) uniqueStrings(rows, APP, 'RUN_ROWS:' + bid, 128);", '']],
  ],
  [
    'solo rows unchecked',
    [[B, "uniqueStrings(run.solo_rows, APP, 'RUN_SOLO_ROWS:' + bid, 128);", '']],
  ],
  [
    'data iso not boolean',
    [[B, "refuse(typeof run.data_iso === 'boolean', 'RUN_DATA_ISO:' + bid);", '']],
  ],
  ['audio not boolean', [[B, "refuse(typeof run.audio === 'boolean', 'RUN_AUDIO:' + bid);", '']]],
  ['guest ram fraction allowed', [[B, ' && v % 1024 === 0)', ')']]],
  [
    'args keys open',
    [
      [
        B,
        "refuse(isObject(args) && subset(args, ARG_KEYS), 'ARGS_KEY:' + bid);",
        "refuse(isObject(args), 'ARGS_KEY:' + bid);",
      ],
    ],
  ],
  [
    'arg words unchecked',
    [[B, 'value.every((w) => isString(w) && WORD.test(w)),\n    `ARGS', 'true,\n    `ARGS']],
  ],
  [
    'arg word count unbounded',
    [[B, 'Array.isArray(value) && value.length <= 16 &&', 'Array.isArray(value) &&']],
  ],
  ['per row not an object', [[B, "refuse(isObject(perRow), 'ARGS_PER_ROW:' + bid);", '']]],
  ['per row name unchecked', [[B, "refuse(APP.test(row), 'ARGS_PER_ROW_NAME:' + bid);", '']]],
  [
    'per row names need not be rows',
    [
      [
        B,
        'if (rows !== null) refuse(rows.includes(row), `ARGS_PER_ROW_UNKNOWN:${bid}:${row}`);',
        '',
      ],
    ],
  ],
  [
    'data iso word allowed',
    [[B, "!flat.includes('--data-iso') && !Object.values", 'true && !Object.values']],
  ],
  [
    'data iso word allowed per row',
    [[B, "&& !Object.values(perRow).some((w) => w.includes('--data-iso')),", '&& true,']],
  ],
  // ---- the linux run
  ['linux run typo allowed', [[B, "refuse(subset(run, LINUX_RUN_KEYS), 'RUN_KEY:' + bid);", '']]],
  [
    'isolation unchecked',
    [[B, "refuse(ISOLATIONS.includes(run.isolation), 'RUN_ISOLATION:' + bid);", '']],
  ],
  [
    'run check picks the wrong platform',
    [
      [
        B,
        "if (b.os === 'windows') checkWindowsRun(bid, b.run);",
        "if (b.os === 'linux') checkWindowsRun(bid, b.run);",
      ],
    ],
  ],
  // ---- the resolver
  [
    'unknown app skipped',
    [[B, "refuse(have.length > 0, 'UNKNOWN_APP:' + name);", 'if (have.length === 0) continue;']],
  ],
  [
    'bad app name accepted',
    [[B, "for (const name of wanted) refuse(APP.test(name), 'BAD_APP:' + name.slice(0, 32));", '']],
  ],
  [
    'os unchecked in the resolver',
    [[B, "refuse(os === null || OSES.includes(os), 'BUNDLE_OS:' + String(os).slice(0, 16));", '']],
  ],
  ['os ignored', [[B, '(os === null || bundles[id].os === os)', 'true']]],
  [
    'ambiguity allowed',
    [
      [
        B,
        "refuse(new Set(have.map((id) => bundles[id].os)).size === 1, 'AMBIGUOUS_APP:' + name);",
        '',
      ],
    ],
  ],
  ['empty request crashes', [[B, 'if (wanted.length === 0) return [];', '']]],
  [
    'cover not checked',
    [
      [
        B,
        'if (!wanted.every((name) => holders[name].some((id) => pick.includes(id)))) continue;',
        '',
      ],
    ],
  ],
  [
    'bytes ignored',
    [
      [
        B,
        'const bytes = pick.reduce((sum, id) => sum + bundles[id].size_bytes, 0);',
        'const bytes = 0;',
      ],
    ],
  ],
  [
    'fewest bundles first',
    [
      [
        B,
        'bytes < best.bytes ||\n        (bytes === best.bytes &&\n          (count < best.pick.length ||',
        'count < best.pick.length ||\n        (count === best.pick.length &&\n          (bytes < best.bytes ||',
      ],
    ],
  ],
  [
    'id tie-break reversed',
    [[B, "pick.join('\\0') < best.pick.join('\\0')", "pick.join('\\0') > best.pick.join('\\0')"]],
  ],
  ['dearest cover wins', [[B, 'bytes < best.bytes ||', 'bytes > best.bytes ||']]],
  ['result unsorted', [[B, 'return [...best.pick];', 'return [...best.pick].reverse();']]],
  [
    'download bytes ignore sizes',
    [[B, 'return ids.reduce((sum, id) => sum + bundles[id].size_bytes, 0);', 'return ids.length;']],
  ],
  // ---- the planner
  [
    'apps and images may be combined',
    [
      [
        A,
        "  if (opts.apps !== undefined && opts.images !== undefined)\n    throw Error('apps and images are alternatives: give one');\n",
        '',
      ],
    ],
  ],
  [
    'image need not be a bundle',
    [
      [
        A,
        "    else if (!Object.hasOwn(bundles?.bundles ?? {}, id))\n      errors.push('image ' + id + ': not a bundle of propimage-bundles.json');\n",
        '',
      ],
    ],
  ],
  [
    'cli apps flag ignored',
    [
      [
        R,
        "else if (args[i] === '--apps') app.apps = need(++i).split(',');",
        "else if (args[i] === '--apps') need(++i);",
      ],
    ],
  ],
];
const selected = process.argv.slice(2);
let failed = false;
const tap = (run) => {
  const out = run.stdout ?? '';
  return {
    kills: [...out.matchAll(/^not ok \d+ - (.+)$/gm)]
      .map((m) => m[1])
      .filter((t) => !t.endsWith('.test.js')),
    complete: /^# tests \d+$/m.test(out) && /^# fail \d+$/m.test(out),
  };
};
const test = () =>
  spawnSync(
    process.execPath,
    ['--no-warnings', '--test', '--test-reporter=tap', '--test-timeout=60000', ...TESTS],
    {
      cwd: here,
      encoding: 'utf8',
      timeout: 300000,
      maxBuffer: 64 * 1024 * 1024,
    },
  );
// the unmutated copy must pass first, or a "kill" proves nothing
const baseline = test();
if (baseline.status !== 0 || !tap(baseline).complete) {
  console.log(
    JSON.stringify({ mutant: 'baseline', ok: false, why: 'the unmutated copy does not pass' }),
  );
  process.exit(1);
}
for (const [name, edits] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const originals = new Map();
  const current = new Map();
  let ambiguous = false;
  for (const [file, before] of edits) {
    const path = here + file;
    const text = originals.get(path) ?? readFileSync(path, 'utf8');
    originals.set(path, text);
    if (text.split(before).length !== 2) ambiguous = true;
  }
  for (const [path, text] of originals) current.set(path, text);
  if (!ambiguous)
    for (const [file, before, after] of edits) {
      const path = here + file;
      current.set(path, current.get(path).split(before).join(after));
    }
  if (ambiguous) {
    console.log(JSON.stringify({ mutant: name, ok: false, why: 'ambiguous or missing' }));
    failed = true;
    continue;
  }
  try {
    for (const [path, text] of current) writeFileSync(path, text);
    const run = test();
    const { kills, complete } = tap(run);
    // The run must have finished (the TAP summary is printed last): a crash or a hang with no result lines is never a kill.
    const ok = run.status !== 0 && kills.length >= 2 && !run.error && complete;
    failed ||= !ok;
    console.log(
      JSON.stringify({
        mutant: name,
        status: run.status,
        kills: kills.length,
        complete,
        ok,
        first: kills.slice(0, 3),
      }),
    );
  } finally {
    for (const [path, text] of originals) writeFileSync(path, text);
  }
}
if (failed) {
  console.log('SURVIVING OR UNDER-KILLED MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
