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
const WIN = 't - bad.first < DEFAULTS.providerGoneWindowSeconds * 1000';
const CNT = 'bad.count < DEFAULTS.providerGoneReadings ||';
const QUIET =
  '!ambiguous ||\n      t - (record.lastPostAt ?? record.started) >= DEFAULTS.providerQuietSeconds * 1000';
const PENDING =
  'if (pendingBoot) {\n        delete record.providerBad;\n        await ledger.save(record);\n        return;\n      }';
const OWN_REPORT =
  "if (error instanceof BrokerError && error.state === 'OWNERSHIP_MISMATCH') throw error;";
const OWN_UPDATE =
  "if (error instanceof BrokerError && error.state === 'OWNERSHIP_MISMATCH') {\n        record.closed";
const SAN =
  'let text = sanitizeBoxText(\n      new TextEncoder().encode(m.slice(0, MESSAGE_INPUT_CHARS)),';
const WORD = "return typeof a === 'string' && /^[a-z_]{1,16}$/.test(a) ? a : 'unknown';";
const LINE_ENDED = 'if (record.ended !== null || record.closeLogged) return;';
// [name, [[file, before, after], ...]]
const MUTANTS = [
  [
    'old behaviour: one non-running reading closes',
    [
      [I, WIN, 'false'],
      [I, CNT, ''],
      [I, QUIET, 'true'],
    ],
  ],
  ['readings counted, no time window', [[I, WIN, 'false']]],
  ['time window, no reading count', [[I, CNT, '']]],
  ['no spacing between counted readings', [[I, 'else if (t - bad.last >= spacing) {', 'else {']]],
  [
    'window default shortened to 60 s',
    [[D, '"providerGoneWindowSeconds": 120', '"providerGoneWindowSeconds": 60']],
  ],
  [
    'reading count default lowered to 2',
    [[D, '"providerGoneReadings": 3', '"providerGoneReadings": 2']],
  ],
  ['ambiguous words need no quiet box (F2)', [[I, QUIET, 'true']]],
  [
    'quiet period shortened to 60 s (F2)',
    [[D, '"providerQuietSeconds": 660', '"providerQuietSeconds": 60']],
  ],
  [
    'every word is ambiguous (F2)',
    [[I, "const DEFINITE = ['exited', 'stopped'];", 'const DEFINITE = [];']],
  ],
  ['no word is ambiguous (F2)', [[I, '!DEFINITE.includes(instance.actual_status)', 'false']]],
  [
    'RUN_GONE skipped while Vast reads non-running (F1)',
    [
      [
        I,
        "if (!pendingBoot && record.resultAt === undefined && record.source !== 'admin') {",
        "if (!pendingBoot && instance.actual_status === 'running' && record.resultAt === undefined && record.source !== 'admin') {",
      ],
    ],
  ],
  [
    'a GitHub failure blocks the window (F1)',
    [[I, "if (instance.actual_status === 'running') throw error;", 'throw error;']],
  ],
  [
    'a GitHub failure is swallowed while running (F1)',
    [[I, "if (instance.actual_status === 'running') throw error;", '']],
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
    'a pending booting reading does not clear the window (F6)',
    [
      [
        I,
        PENDING,
        'if (pendingBoot) {\n        await ledger.save(record);\n        return;\n      }',
      ],
    ],
  ],
  [
    'the counter is not saved (F6)',
    [
      [
        I,
        '      await ledger.save(record);\n      return;\n    }\n    delete record.providerBad;',
        '      return;\n    }\n    delete record.providerBad;',
      ],
    ],
  ],
  [
    'report() closes on a non-running status (old behaviour)',
    [
      [
        I,
        "if (instance !== undefined && instance.actual_status !== 'running') {\n            r.providerAnomalies",
        "if (instance !== undefined && instance.actual_status !== 'running') {\n            throw Error('NOT_LIVE');\n            r.providerAnomalies",
      ],
    ],
  ],
  [
    'report() does not count the anomaly',
    [
      [
        I,
        "if (instance !== undefined && instance.actual_status !== 'running') {\n            r.providerAnomalies = (r.providerAnomalies ?? 0) + 1;",
        "if (instance !== undefined && instance.actual_status !== 'running') {\n            ",
      ],
    ],
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
        PENDING,
        PENDING +
          "\n      if (record.firstReportAt === undefined) {\n        await close(record, vast, 'ENDED');\n        return;\n      }",
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
        "            r.providerMsg = '-';\n            throw Error('NOT_LIVE');",
        "            r.providerMsg = '-';\n            return await ledger.save(r);",
      ],
    ],
  ],
  [
    'a provider API error closes at once in update() (F3)',
    [[I, 'if (badReading(record, true)) {', 'if (true) {']],
  ],
  [
    'a provider API error closes at once in report() (F3)',
    [[I, '            instance = undefined;\n', '            throw error;\n']],
  ],
  [
    'an ownership mismatch is tolerated in update() (F3)',
    [[I, OWN_UPDATE, 'if (false) {\n        record.closed']],
  ],
  ['an ownership mismatch is tolerated in report() (F3)', [[I, OWN_REPORT, '']]],
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
  ['closeLine repeats', [[I, LINE_ENDED, 'if (record.ended !== null) return;']]],
  ['closeLine writes on ended records', [[I, LINE_ENDED, 'if (record.closeLogged) return;']]],
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
    'the persistent API error close writes no line',
    [
      [
        I,
        "        await closeLine(record, 'UNAVAILABLE');\n        throw Error('UNAVAILABLE');\n      }\n      await ledger.save(record);",
        "        throw Error('UNAVAILABLE');\n      }\n      await ledger.save(record);",
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
  ['raw status printed as unknown', [[I, WORD, "return 'unknown';"]]],
  ['raw status not restricted', [[I, WORD, 'return String(a);']]],
  [
    'status_msg not sanitised',
    [[I, SAN, "let text = String(m) + sanitizeBoxText(\n      new TextEncoder().encode(''),"]],
  ],
  [
    'status_msg cut before it is sanitised (F5)',
    [[I, 'm.slice(0, MESSAGE_INPUT_CHARS)', 'm.slice(0, 1024)']],
  ],
  [
    'a token cut by the input bound is kept (F5)',
    [[I, "if (cut) text = text.replace(/[A-Za-z0-9_-]+$/, '');", '']],
  ],
  [
    'status_msg keeps the instance id and label (F6)',
    [
      [
        I,
        '[...secrets, record.id, record.label, record.nonce],\n      { tail: false },',
        '[...secrets, record.nonce],\n      { tail: false },',
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
