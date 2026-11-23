// Mutation check for the gpu-vast hardening: the commit-gap release, the replacement deadline extension, the kvm device
// signature and the gate-reuse stage rule (DEPLOY.md, "gpu-vast hardening"). GPU-free. Each mutant is applied in a
// temporary copy of the tree (mutant-copy.mjs), the tests of the change run, and every mutant must make at least TWO
// named tests fail; dropping the narrowing of the commit gap is the first mutant.
//   node ci/vast-broker/sha-gap-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tempCopy } from './mutant-copy.mjs';

const here = tempCopy();
const TESTS = [
  'sha-gap-provider-fault',
  'provider-rules',
  'replacement-deadline',
  'provider-kvm-device',
  'green-provider',
  'gate-reuse',
].map((name) => here + 'test/' + name + '.test.js');
const P = 'src/provider-policy.js',
  I = 'src/index.js',
  L = 'src/ledger.js',
  D = 'defaults.json',
  G = 'gate_reuse.py';
const UNUSED =
  "export const unusedRental = (record) =>\n  record.verdict === 'PROVIDER_FAULT' &&\n  record.firstReportAt === undefined &&\n  record.replacementExhausted === true;";
const EXT = 'return Math.max(earlier, Math.min(cap, lost, room));';
// Mutants one scenario kills: close() turns a box that reported into FAIL, so no end to end run reaches a
// PROVIDER_FAULT with a report; a shorter or summed extension is visible only across legs; the foreign marker and the
// chain end in exhaust() each have a single path.
const SINGLE = new Set([
  'commit gap released for any verdict',
  'commit gap released after a box report',
  'extension may shrink',
  'marker of another run key deleted',
  'release in exhaust() removed',
  'extensions summed over legs',
]);
// [name, [[file, before, after], ...]]
const MUTANTS = [
  [
    'commit gap released for every rental (no narrowing)',
    [[P, UNUSED, 'export const unusedRental = () => true;']],
  ],
  [
    'commit gap released for any verdict',
    [[P, "record.verdict === 'PROVIDER_FAULT' &&\n  record.firstReportAt", 'record.firstReportAt']],
  ],
  [
    'commit gap released after a box report',
    [
      [
        P,
        'record.firstReportAt === undefined &&\n  record.replacementExhausted',
        'record.replacementExhausted',
      ],
    ],
  ],
  [
    'commit gap released while a replacement is allowed',
    [
      [
        P,
        'record.firstReportAt === undefined &&\n  record.replacementExhausted === true;',
        'record.firstReportAt === undefined;',
      ],
    ],
  ],
  ['commit gap never released', [[P, UNUSED, 'export const unusedRental = () => false;']]],
  [
    'release ignores the verdict test (index)',
    [
      [
        I,
        'if (unusedRental(record)) await clearFailedSha(record);',
        'await clearFailedSha(record);',
      ],
    ],
  ],
  [
    'marker of another run key deleted',
    [
      [
        I,
        'if (cached?.key === record.key) await ledger.storage.delete',
        'if (cached) await ledger.storage.delete',
      ],
    ],
  ],
  [
    'spend row released with the marker',
    [
      [
        I,
        'if (unusedRental(record)) await clearFailedSha(record);',
        "if (unusedRental(record)) {\n      await clearFailedSha(record);\n      await ledger.storage.delete('cost:' + record.key);\n    }",
      ],
    ],
  ],
  [
    'release in cleanup() removed',
    [[I, '    await releaseUnusedSha(record);\n    console.log', '    console.log']],
  ],
  ['release in exhaust() removed', [[I, '      await releaseUnusedSha(r);\n', '']]],
  ['extension has no cap', [[P, EXT, 'return Math.max(earlier, Math.min(lost, room));']]],
  [
    'extension ignores the runner room',
    [[P, EXT, 'return Math.max(earlier, Math.min(cap, lost));']],
  ],
  ['extension ignores the time lost', [[P, EXT, 'return Math.max(earlier, Math.min(cap, room));']]],
  ['extension may shrink', [[P, EXT, 'return Math.min(cap, lost, room);']]],
  [
    'extension default 0',
    [[D, '"replacementExtensionMinutes": 25', '"replacementExtensionMinutes": 0']],
  ],
  [
    'extension default 90',
    [[D, '"replacementExtensionMinutes": 25', '"replacementExtensionMinutes": 90']],
  ],
  [
    'reservation does not check the extension',
    [[L, '!validExtension(record, prior, cfg.provider) ||', 'false ||']],
  ],
  [
    'extensions summed over legs',
    [[I, 'retrySource.hardDeadline + shift;', 'retrySource.hardDeadline + retryExtension;']],
  ],
  [
    'soft deadline not moved',
    [
      [
        I,
        'r.softDeadline = retrySource.softDeadline + shift;',
        'r.softDeadline = retrySource.softDeadline;',
      ],
    ],
  ],
  [
    'kvm device signature removed',
    [[D, '"reason": "KVM_DEVICE_ERROR"', '"reason": "KVM_DEVICE_ERRORS_OFF"']],
  ],
  [
    'kvm device signature matches any daemon device error',
    [[D, '          "/dev/kvm"\n', '          "/dev/"\n']],
  ],
  ['replay takes only the old stage count', [[G, 'stages=[0-9]{1,2}"', 'stages=4"']]],
  ['replay allows a repeated stage', [[G, 'or len(set(stages)) != planned\n', '\n']]],
  [
    'replay ignores the base order',
    [[G, 'or tuple(stages[: len(BASE_STAGES)]) != BASE_STAGES\n', '\n']],
  ],
  [
    'replay has no stage limit',
    [[G, 'not len(BASE_STAGES) <= planned <= MAX_STAGES\n        or ', '']],
  ],
  [
    'replay stage names unrestricted',
    [[G, 'STAGE_NAME = r"[a-z][a-z0-9_]{0,39}"', 'STAGE_NAME = r"[^ ]+"']],
  ],
  ['replay allows many rentals', [[G, 'MAX_RENTALS = 8', 'MAX_RENTALS = 100']]],
];

const selected = process.argv.slice(2);
let failed = false;
for (const [name, edits] of MUTANTS) {
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
      ['--no-warnings', '--test', '--test-reporter=tap', '--test-timeout=60000', ...TESTS],
      { cwd: here, encoding: 'utf8', timeout: 300000, maxBuffer: 64 * 1024 * 1024 },
    );
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)]
      .map((m) => m[1])
      .filter((t) => !t.endsWith('.test.js'));
    // The run must have finished (the TAP summary is printed last): a crash or a hang is never a kill.
    const complete =
      /^# tests \d+$/m.test(run.stdout ?? '') && /^# fail \d+$/m.test(run.stdout ?? '');
    const ok =
      run.status !== 0 && kills.length >= (SINGLE.has(name) ? 1 : 2) && !run.error && complete;
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
