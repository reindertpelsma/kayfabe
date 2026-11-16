// Mutation check for the Worker's proprietary-image configuration (src/propimages.js, propimages.json, the /bench-data
// allow-list in src/benchdata.js), GPU-free. Each mutant is applied in a temporary COPY of the tree (mutant-copy.mjs), the file
// test/propimages.test.js runs, and the copy is restored before the next mutant. Every mutant must make at least TWO named tests
// fail. Run with no other test running here:
//   node ci/vast-broker/propimages-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tempCopy } from './mutant-copy.mjs';

const here = tempCopy();
const TESTS = [here + 'test/propimages.test.js', here + 'test/bandwidth.test.js'];
const S = 'src/propimages.js',
  C = 'propimages.json',
  B = 'src/benchdata.js';
// [name, [[file, before, after], ...]]  or  [name, { file, change(json) }] for an edit of a JSON data file
const BW = 'src/bandwidth.js',
  BJ = 'bandwidth.json';
const MUTANTS = [
  [
    'strict flag dropped from the part',
    [[S, '    propimages_strict: true,', '    propimages_strict: false,']],
  ],
  [
    'non-strict config accepted',
    [[S, "bad('NOT_STRICT', value.strict === true);", "bad('NOT_STRICT', true);"]],
  ],
  ['config file made non-strict', [[C, '"strict": true,', '"strict": false,']]],
  [
    'sha256 pin not checked',
    [[S, "typeof o.sha256 === 'string' && SHA256.test(o.sha256)", 'true']],
  ],
  ['size not checked', [[S, 'Number.isSafeInteger(o.size) && o.size > 0', 'true']]],
  [
    'sha256 pin left out of the disk',
    [
      [
        S,
        "disk: { from: 'pin', ref: object.ref, sha256: object.sha256 },",
        "disk: { from: 'pin', ref: object.ref },",
      ],
    ],
  ],
  [
    'object key sent as the ref',
    [
      [
        S,
        "disk: { from: 'pin', ref: object.ref, sha256: object.sha256 },",
        "disk: { from: 'pin', ref: object.object, sha256: object.sha256 },",
      ],
    ],
  ],
  ['lane not checked', [[S, 'LANES.includes(row.lane)', 'true']]],
  [
    'unmeasured family keeps the configured lanes',
    [[S, "lane: measured ? row.lane : 'discovery'", 'lane: row.lane']],
  ],
  [
    'every family counts as measured',
    [[S, 'const measured = img.measured_families.includes(family);', 'const measured = true;']],
  ],
  [
    'slot ignored: every image on every box',
    [
      [
        S,
        'chosen = value.images.filter((img) => boxSlot(img) === slot);',
        'chosen = value.images;',
      ],
    ],
  ],
  ['swap never applied', [[S, 'const swap = value.swap !== rotation;', 'const swap = false;']]],
  [
    'rotation by release number ignored',
    [[S, 'const swap = value.swap !== rotation;', 'const swap = value.swap;']],
  ],
  ['config swap ignored', [[S, 'const swap = value.swap !== rotation;', 'const swap = rotation;']]],
  [
    'release number parity inverted',
    [[S, 'const rotation = releaseSeq % 2 === 1;', 'const rotation = releaseSeq % 2 === 0;']],
  ],
  [
    'release number not required',
    [
      [
        S,
        "if (!Number.isSafeInteger(releaseSeq) || releaseSeq < 0) throw Error('propimages releaseSeq');",
        '',
      ],
    ],
  ],
  ['single die still splits the images', [[S, 'if (count === 1) {', 'if (false) {']]],
  ['die place not taken from the matrix order', [[S, 'return at + 1;', 'return 1;']]],
  [
    'release profile not required',
    [[S, "if (profile !== PROPIMAGES_PROFILE) throw Error('propimages release only');", '']],
  ],
  [
    'tier not checked',
    [[S, "if (!PROPIMAGES_TIERS.includes(tierName)) throw Error('propimages tier');", '']],
  ],
  [
    'unknown image label ignored',
    [
      [
        S,
        "if (img === undefined) throw Error('propimages unknown image:' + String(label).slice(0, 32));",
        'if (img === undefined) return { label, disk: {}, apps: {}, measured_families: [] };',
      ],
    ],
  ],
  ['duplicate image selection accepted', [[S, ' || new Set(images).size !== images.length', '']]],
  ['default on', [[S, 'if (propimages !== false) {', 'if (true) {']]],
  [
    'flag accepted when it is not exactly true',
    [[S, "if (propimages !== true) throw Error('propimages flag');", '']],
  ],
  [
    'driver-matrix leg accepted',
    [
      [
        S,
        "if (tierOptions.releaseMatrix !== undefined) throw Error('instruction release matrix');",
        '',
      ],
    ],
  ],
  ['windows runs read wrongly', [[S, "env.KF_WINDOWS_RUNS === '2' ? 2 : 1", '1']]],
  [
    'linux apps profile read wrongly',
    [
      [
        S,
        "env.KF_LINUX_APPS === '1' ? 'middle' : null",
        "env.KF_LINUX_APPS === '1' ? 'large' : null",
      ],
    ],
  ],
  [
    'canonical keys not sorted',
    [
      [
        S,
        '          Object.keys(v)\n            .sort()\n            .map(',
        '          Object.keys(v)\n            .map(',
      ],
    ],
  ],
  [
    'canonical indent changed',
    [[S, 'JSON.stringify(sorted(doc), null, 2)', 'JSON.stringify(sorted(doc), null, 4)']],
  ],
  ['duplicate label accepted', [[S, '!labels.has(img.label)', 'true']]],
  ['duplicate object id accepted', [[S, '!objects.has(o.id)', 'true']]],
  [
    'two images in one slot accepted',
    [[S, 'SLOTS.includes(img.slot) && !slots.has(img.slot)', 'SLOTS.includes(img.slot)']],
  ],
  ['image may name any object', [[S, "objects.get(img.object)?.role === 'image'", 'true']]],
  ['runner registry not consulted', [[S, 'if (registry !== undefined)', 'if (false)']]],
  ['excluded application may also be listed', [[S, ' && !(id in apps)', '']]],
  [
    'private image added to the allow-list',
    [
      [
        B,
        "  ['winapps-open.manifest.json', 'winapps-open/manifest.json'],",
        "  ['winapps-open.manifest.json', 'winapps-open/manifest.json'],\n  ['propimage-kfimg-20261112.zip', 'windows-proprietary/kfimg-20261112/image-v2.zip'],",
      ],
    ],
  ],
  [
    'image A sha256 changed in the config',
    [
      [
        C,
        'cddff40a501d8135c6c15ef7267b3b838304e5d6f089502707bce78712cb11b3',
        'cddff40a501d8135c6c15ef7267b3b838304e5d6f089502707bce78712cb11b4',
      ],
    ],
  ],
  [
    'a gate row moved to discovery in the config',
    [C, { change: (d) => (d.images[0].apps.edge.lane = 'discovery') }],
  ],
  [
    'image B marked measured on blackwell in the config',
    [C, { change: (d) => d.images[1].measured_families.push('blackwell') }],
  ],
  [
    'image slots exchanged in the config',
    [C, { change: (d) => ([d.images[0].slot, d.images[1].slot] = [2, 1]) }],
  ],
  ['swap switched on in the config', [C, { change: (d) => (d.swap = true) }]],
  [
    'wukong listed in the config',
    [C, { change: (d) => (d.images[0].apps.wukong = { lane: 'discovery', basis: 'x' }) }],
  ],
  // the bandwidth estimate (src/bandwidth.js, bandwidth.json)
  [
    'margin not applied',
    [
      [
        BW,
        'const scaled = (n) => Math.ceil((n * (PERCENT + margin)) / PERCENT);',
        'const scaled = (n) => n;',
      ],
    ],
  ],
  [
    'per-image lines counted once for no image',
    [
      [
        BW,
        'for (const label of entry.per_image ? images : [undefined]) {',
        'for (const label of [undefined]) {',
      ],
    ],
  ],
  ['conditions of an entry ignored', [[BW, 'if (!matches(entry.when, ctx)) return;', '']]],
  [
    'skipped stage still counted',
    [
      [
        BW,
        'const chosen = plan.filter((n) => (!only.length || only.includes(n)) && !skip.includes(n));',
        'const chosen = plan;',
      ],
    ],
  ],
  [
    'pinned size not read from the pin',
    [
      [
        BW,
        "return { bytes: object.size, from: 'propimages.json:' + id };",
        "return { bytes: 1, from: 'propimages.json:' + id };",
      ],
    ],
  ],
  ['r2 list ignored', [[BW, 'if (count(r2Sizes[name])) return', 'if (false) return']]],
  ['upload not counted', [[BW, 'up: upload.up_bytes,', 'up: 0,']]],
  [
    'image stage planned without an image list',
    [
      [
        BW,
        "if ((i.propimages ?? []).length > 0 || (i.tier === 'large' && only.includes('propimage_apps')))",
        'if (true)',
      ],
    ],
  ],
  [
    'default image not counted for the manual stage',
    [[BW, "chosen.includes('propimage_apps') ? table.default_images : []", '[]']],
  ],
  [
    'repo size passed in ignored',
    [[BW, "if (entry.id === 'fixed.repo_clone' && count(repoBytes))", 'if (false)']],
  ],
  [
    'image size copied into the table',
    [[BJ, '"size_from": "propimage:image"', '"down_bytes": 50496618964']],
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
    { cwd: here, encoding: 'utf8', timeout: 300000, maxBuffer: 64 * 1024 * 1024 },
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
  if (typeof edits[1]?.change === 'function') {
    // an edit of a JSON data file: parse, change, write back
    const path = here + edits[0];
    const text = readFileSync(path, 'utf8');
    originals.set(path, text);
    const doc = JSON.parse(text);
    edits[1].change(doc);
    current.set(path, JSON.stringify(doc, null, 2) + '\n');
  } else {
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
