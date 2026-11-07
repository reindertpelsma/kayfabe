// Mutation check for the evidence route (src/evidence.js, and its wiring in src/index.js and
// src/routes.js), GPU-free. Each mutant is applied in place (one or more exact-once replacements),
// test/evidence.test.js runs, and every file is restored before the next mutant (also on error).
// Every mutant must make at least TWO named tests fail. Run with no other test running here:
//   node ci/vast-broker/evidence-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const TESTS = [here + 'test/evidence.test.js'];
const E = 'src/evidence.js',
  I = 'src/index.js',
  R = 'src/routes.js';
// [name, [[file, before, after], ...]]
const MUTANTS = [
  [
    'failed write keeps the flag',
    [
      [
        I,
        "          if (ask.state === 'FAILED') delete r.evidence;\n",
        "          if (ask.state === 'FAILED') r.evidence.at = r.evidence.at;\n",
      ],
    ],
  ],
  [
    'stored flag never set',
    [
      [
        I,
        "          else r.evidence = { state: 'STORED', at: now(), sha256: ask.sha256 ?? null };",
        '          else r.evidence.at = r.evidence.at;',
      ],
    ],
  ],
  [
    'flag conflict answers 200',
    [
      [
        I,
        "if (prior?.state === 'STORED') refuse('EVIDENCE_EXISTS', 409);",
        "if (prior?.state === 'STORED') refuse('EVIDENCE_EXISTS', 200);",
      ],
    ],
  ],
  [
    'ledger does not verify the run token',
    [
      [
        I,
        "          !(await tokens.verify(bearer, r)) ||\n          r.ended !== null ||\n          r.closed ||\n          now() >= r.reportExpiry * 1000\n        )\n          refuse('UNAUTHORIZED', 401);\n        if (!evidenceEnabled",
        "          false ||\n          r.ended !== null ||\n          r.closed ||\n          now() >= r.reportExpiry * 1000\n        )\n          refuse('UNAUTHORIZED', 401);\n        if (!evidenceEnabled",
      ],
    ],
  ],
  [
    'ledger accepts a closed rental',
    [
      [
        I,
        "          r.closed ||\n          now() >= r.reportExpiry * 1000\n        )\n          refuse('UNAUTHORIZED', 401);\n        if (!evidenceEnabled",
        "          false ||\n          now() >= r.reportExpiry * 1000\n        )\n          refuse('UNAUTHORIZED', 401);\n        if (!evidenceEnabled",
      ],
    ],
  ],
  [
    'gateway reads without a length check',
    [[I, 'const length = evidenceLength(request);', 'const length = 1000;']],
  ],
  [
    'gateway holds any number of uploads',
    [[I, "if (evidenceHeld >= 2) refuse('EVIDENCE_BUSY', 503);", '']],
    // Regression (integration 2026-11-06): the cap test itself must fail cleanly on this mutant;
    // under Node 24 it used to wait forever, so the file never finished and no kill was counted.
    'evidence gateway: at most two uploads are held in one isolate, the third is 503',
  ],
  ['gateway never releases its held count', [[I, '      evidenceHeld--;\n', '']]],
  [
    'ledger open reservations unbounded',
    [[I, "if (evidenceOpen.size >= EVIDENCE_MAX_INFLIGHT) refuse('EVIDENCE_BUSY', 503);", '']],
  ],
  [
    'ledger never purges stale reservations',
    [[I, 'if (now() - t >= EVIDENCE_PENDING_MS) evidenceOpen.delete(k);', '']],
  ],
  [
    'settle keeps the reservation open',
    [[I, '    evidenceOpen.delete(key);\n    try {', '    try {']],
  ],
  [
    'request count not limited',
    [
      [
        I,
        '        await rate(r, cfg);\n        const prior = r.evidence;',
        '        const prior = r.evidence;',
      ],
    ],
  ],
  [
    'key is not per attempt',
    [
      [
        E,
        "const base = profile + '/' + m[1] + '/' + m[2] + (retries ? '-r' + retries : '');",
        "const base = profile + '/' + m[1] + (retries ? '-r' + retries : '');",
      ],
    ],
  ],
  ['replacement rental shares the key', [[E, " + (retries ? '-r' + retries : '');", ';']]],
  [
    'ledger flag not checked',
    [
      [
        I,
        "        if (prior?.state === 'STORED') refuse('EVIDENCE_EXISTS', 409);\n        if (prior && now() - prior.at < EVIDENCE_PENDING_MS) refuse('EVIDENCE_PENDING', 409);\n",
        '',
      ],
    ],
  ],
  // Review of #274, R1 (integration 2026-11-06): PENDING is not reported as stored.
  [
    'pending answered as stored',
    [[I, "refuse('EVIDENCE_PENDING', 409)", "refuse('EVIDENCE_EXISTS', 409)"]],
  ],
  ['pending flag never expires', [[I, 'now() - prior.at < EVIDENCE_PENDING_MS', 'true']]],
  [
    'pending flag blocks nobody',
    [
      [
        I,
        "        if (prior && now() - prior.at < EVIDENCE_PENDING_MS) refuse('EVIDENCE_PENDING', 409);\n",
        '',
      ],
    ],
  ],
  ['head check removed', [[E, "typeof bucket.head === 'function' &&", 'false &&']]],
  [
    'gateway evidence limit removed',
    [
      [
        I,
        'if (sent >= 6 || (!evidenceCounts.has(key) && evidenceCounts.size >= 1024))',
        'if (false)',
      ],
    ],
  ],
  ['gateway evidence limit too high', [[I, 'if (sent >= 6 ||', 'if (sent >= 600 ||']]],
  [
    'conflict answers 200',
    [[E, "refuse('EVIDENCE_EXISTS', 409)", "refuse('EVIDENCE_EXISTS', 200)"]],
  ],
  [
    'gzip magic unchecked',
    [
      [
        E,
        "if (body[0] !== 0x1f || body[1] !== 0x8b) refuse('BAD_REQUEST', 400, 'EVIDENCE_FORMAT');",
        '',
      ],
    ],
  ],
  [
    'declared sha not compared',
    [[E, "if (sha256 !== claimed) refuse('BAD_REQUEST', 400, 'EVIDENCE_SHA256');", '']],
  ],
  [
    'body length vs declared unchecked',
    [
      [
        E,
        "if (body.length !== length || body.length < 20) refuse('BAD_REQUEST', 400, 'EVIDENCE_LENGTH');",
        '',
      ],
    ],
  ],
  [
    'cap raised',
    [
      [
        E,
        'export const EVIDENCE_MAX_BYTES = 8 * 1024 * 1024;',
        'export const EVIDENCE_MAX_BYTES = 64 * 1024 * 1024;',
      ],
    ],
  ],
  [
    'cap not enforced before reading',
    [[E, "if (Number(raw) > EVIDENCE_MAX_BYTES) refuse('PAYLOAD_TOO_LARGE', 413);", '']],
  ],
  [
    'declared length not required',
    [
      [
        E,
        "if (raw === null || !/^[1-9][0-9]{0,9}$/.test(raw)) refuse('LENGTH_REQUIRED', 411);",
        '',
      ],
    ],
  ],
  [
    'disabled bucket answers 503',
    [
      [
        I,
        "if (!evidenceEnabled(env)) refuse('EVIDENCE_DISABLED', 404);",
        "if (!evidenceEnabled(env)) refuse('EVIDENCE_DISABLED', 503);",
      ],
    ],
  ],
  [
    'disabled check removed',
    [[I, "if (!evidenceEnabled(env)) refuse('EVIDENCE_DISABLED', 404);", '']],
  ],
  [
    'binding check accepts anything',
    [[E, "typeof env.EVIDENCE_BUCKET.put === 'function'", 'true']],
  ],
  [
    'gateway does not authenticate the route',
    [
      [
        I,
        'if (/^\\/(?:report|run-report|kill|blob-url|bench-data|evidence)\\//.test(path)) {',
        'if (/^\\/(?:report|run-report|kill|blob-url|bench-data)\\//.test(path)) {',
      ],
    ],
  ],
  [
    'route not recognised as a box route',
    [[R, '/^\\/(?:report|run-report|kill|evidence)\\/', '/^\\/(?:report|run-report|kill)\\/']],
  ],
  [
    'profile ignored in the key',
    [[E, 'const profile = budgetClass(record);', "const profile = 'check';"]],
  ],
  ['scan field not required clean', [[E, "meta.scan !== 'clean' ||", '']]],
  [
    'meta accepts extra keys',
    [
      [
        E,
        "const own = (object, keys) =>\n  object !== null &&\n  typeof object === 'object' &&\n  !Array.isArray(object) &&\n  Object.keys(object).every((k) => keys.includes(k));",
        'const own = () => true;',
      ],
    ],
  ],
  ['section names unchecked', [[E, '!SECTION.test(s.name) ||', '']]],
  [
    'manifest holds the box token header',
    [
      [
        E,
        'scrub: meta.scrub,',
        "scrub: meta.scrub, authorization: request.headers.get('Authorization'),",
      ],
    ],
  ],
  [
    'bucket failure is a success',
    [[E, "refuse('UNAVAILABLE', 503, undefined, 'evidence_put');", '']],
  ],
  [
    'manifest failure is fatal',
    [[E, '    manifestStored = false;', "    refuse('UNAVAILABLE', 503);"]],
  ],
  ['content type not gzip', [[E, "httpMetadata: { contentType: 'application/gzip' },", '']]],
  [
    'stored size from the header',
    [[E, 'size: body.length,\n    uploaded_at', 'size: length + 1,\n    uploaded_at']],
  ],
];

const selected = process.argv.slice(2);
let failed = false;
for (const [name, edits, mustKill] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const originals = new Map();
  let ambiguous = false;
  for (const [file, before] of edits) {
    const path = here + file;
    const text = originals.get(path) ?? readFileSync(path, 'utf8');
    originals.set(path, text);
    if (text.split(before).length !== 2) ambiguous = true;
  }
  if (ambiguous) {
    console.log(JSON.stringify({ mutant: name, ok: false, why: 'ambiguous or missing' }));
    failed = true;
    continue;
  }
  try {
    const current = new Map(originals);
    for (const [file, before, after] of edits) {
      const path = here + file;
      current.set(path, current.get(path).split(before).join(after));
    }
    for (const [path, text] of current) writeFileSync(path, text);
    const run = spawnSync(
      process.execPath,
      // A per-test timeout turns a hang into a `not ok` line instead of a dead file (integration
      // 2026-11-06: a hung test under Node 24 left no TAP result at all).
      ['--no-warnings', '--test', '--test-reporter=tap', '--test-timeout=60000', ...TESTS],
      {
        cwd: here,
        encoding: 'utf8',
        timeout: 300000,
        maxBuffer: 64 * 1024 * 1024,
      },
    );
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)]
      .map((m) => m[1])
      .filter((t) => !t.endsWith('.test.js'));
    // The run must have finished (the TAP summary is printed last): a crash or a hang with no
    // result lines is never a kill. A named killer, when given, must be among the kills.
    const complete =
      /^# tests \d+$/m.test(run.stdout ?? '') && /^# fail \d+$/m.test(run.stdout ?? '');
    const ok =
      run.status !== 0 &&
      kills.length >= 2 &&
      !run.error &&
      complete &&
      (mustKill === undefined || kills.includes(mustKill));
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
