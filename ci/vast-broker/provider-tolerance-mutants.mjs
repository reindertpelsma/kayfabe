// Mutation check for the provider-status tolerance and the close lines (src/index.js update(),
// report(), close(), closeLine(); defaults.json), GPU-free. STATUS: LIVE, 2026-11-06. Each mutant is
// applied in place (exact-once replacements), test/provider-tolerance.test.js runs, and every file is
// restored before the next mutant (also on error). Every mutant must make at least TWO named tests
// fail; the OLD behaviour (close on one reading) is the first mutant. Run with nothing else touching
// ci/vast-broker/src:
//   node ci/vast-broker/provider-tolerance-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const TESTS = [here + 'test/provider-tolerance.test.js'];
const I = 'src/index.js',
  D = 'defaults.json';
const COUNT =
  'bad.count >= DEFAULTS.providerGoneReadings &&\n        t - bad.first >= DEFAULTS.providerGoneWindowSeconds * 1000';
// [name, [[file, before, after], ...]]
const MUTANTS = [
  ['old behaviour: one non-running reading closes', [[I, COUNT, 'true']]],
  [
    'readings counted, no time window',
    [[I, 't - bad.first >= DEFAULTS.providerGoneWindowSeconds * 1000', 'true']],
  ],
  ['time window, no reading count', [[I, 'bad.count >= DEFAULTS.providerGoneReadings &&', '']]],
  ['no spacing between counted readings', [[I, 'else if (t - bad.last >= spacing) {', 'else {']]],
  [
    'window default shortened to 60 s',
    [[D, '"providerGoneWindowSeconds": 120', '"providerGoneWindowSeconds": 60']],
  ],
  [
    'reading count default lowered to 2',
    [[D, '"providerGoneReadings": 3', '"providerGoneReadings": 2']],
  ],
  [
    'update() ignores a box POST newer than the window',
    [[I, '(record.lastPostAt !== null && record.lastPostAt > bad.first)', 'false']],
  ],
  [
    'report() does not clear the window',
    [
      [
        I,
        '      r.lastPostAt = now();\n      delete r.providerBad;\n',
        '      r.lastPostAt = now();\n',
      ],
    ],
  ],
  [
    'a running reading does not clear the window',
    [
      [
        I,
        '    delete record.providerBad;\n    record.liveCheckedAt = now();',
        '    record.liveCheckedAt = now();',
      ],
    ],
  ],
  [
    'report() closes on a non-running status (old behaviour)',
    [
      [
        I,
        "if (instance.actual_status !== 'running') {\n            r.providerAnomalies",
        "if (instance.actual_status !== 'running') {\n            throw Error('NOT_LIVE');\n            r.providerAnomalies",
      ],
    ],
  ],
  [
    'report() does not count the anomaly',
    [[I, 'r.providerAnomalies = (r.providerAnomalies ?? 0) + 1;', '']],
  ],
  [
    'booting offline is not pending',
    [
      [
        I,
        "['created', 'loading', 'starting', 'offline'].includes(i.actual_status)",
        "['created', 'loading', 'starting'].includes(i.actual_status)",
      ],
    ],
  ],
  [
    'booting non-running closes at once (no tolerance while booting)',
    [
      [
        I,
        'if (record.firstReportAt === undefined && pendingInstance(instance)) {\n        delete record.providerBad;\n        await ledger.save(record);\n        return;\n      }',
        "if (record.firstReportAt === undefined && pendingInstance(instance)) {\n        delete record.providerBad;\n        await ledger.save(record);\n        return;\n      }\n      if (record.firstReportAt === undefined) {\n        await close(record, vast, 'ENDED');\n        return;\n      }",
      ],
    ],
  ],
  [
    'absence is not special in update()',
    [
      [
        I,
        '    if (instance === null) {\n      // Verified absence',
        '    if (instance === undefined) {\n      // Verified absence',
      ],
    ],
  ],
  [
    'absence is tolerated in report()',
    [
      [
        I,
        "          if (instance === null) {\n            r.providerRaw = 'absent';\n            r.providerMsg = '-';\n            throw Error('NOT_LIVE');\n          }",
        "          if (instance === null) {\n            r.providerRaw = 'absent';\n            r.providerMsg = '-';\n            return await ledger.save(r);\n          }",
      ],
    ],
  ],
  [
    'close() writes no line',
    [
      [
        I,
        '    await closeLine(record, reason);\n    if (record.ended === null) await cleanup(record, vast);',
        '    if (record.ended === null) await cleanup(record, vast);',
      ],
    ],
  ],
  [
    'closeLine repeats',
    [
      [
        I,
        'if (record.ended !== null || record.closeLogged) return;',
        'if (record.ended !== null) return;',
      ],
    ],
  ],
  [
    'closeLine writes on ended records',
    [
      [
        I,
        'if (record.ended !== null || record.closeLogged) return;',
        'if (record.closeLogged) return;',
      ],
    ],
  ],
  [
    'stop-all writes no close line',
    [
      [
        I,
        "        await ledger.save(r);\n        await closeLine(r, 'STOPPED');",
        '        await ledger.save(r);',
      ],
    ],
  ],
  [
    'provider_error writes no close line',
    [
      [
        I,
        "      await closeLine(record, 'UNAVAILABLE');\n      throw Error('UNAVAILABLE');",
        "      throw Error('UNAVAILABLE');",
      ],
    ],
  ],
  [
    'absent writes no close line',
    [
      [
        I,
        "      await closeLine(record, 'GONE');\n      await ledger.settle(record, now());\n      return;\n    }\n    await checkContract",
        '      await ledger.settle(record, now());\n      return;\n    }\n    await checkContract',
      ],
    ],
  ],
  [
    'raw status printed as unknown',
    [
      [
        I,
        "return typeof a === 'string' && /^[a-z_]{1,16}$/.test(a) ? a : 'unknown';",
        "return 'unknown';",
      ],
    ],
  ],
  [
    'raw status not restricted',
    [
      [
        I,
        "return typeof a === 'string' && /^[a-z_]{1,16}$/.test(a) ? a : 'unknown';",
        'return String(a);',
      ],
    ],
  ],
  [
    'status_msg not sanitised',
    [
      [
        I,
        '    const text = sanitizeBoxText(\n      new TextEncoder().encode(m.slice(0, 1024)),',
        "    const text = String(m) + sanitizeBoxText(\n      new TextEncoder().encode(''),",
      ],
    ],
  ],
  [
    'status_msg not bounded',
    [
      [
        I,
        'Array.from(text).slice(0, DEFAULTS.providerMessageChars).join(',
        'Array.from(text).join(',
      ],
    ],
  ],
  [
    'status_msg not stored for the close line',
    [
      [
        I,
        '    record.providerMsg = providerMessage(record, instance);\n    if (record.providerState',
        '    record.providerMsg = undefined;\n    if (record.providerState',
      ],
    ],
  ],
  ['flaps not counted', [[I, 'record.providerFlaps = (record.providerFlaps ?? 0) + 1;', '']]],
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
