// Mutation check for the coordinator identity (worker.js), GPU-free. Each mutant is one exact-once
// replacement; test/coordinator.test.mjs runs; the file is restored (also on error). Every mutant must
// make at least TWO named tests fail:   node ci/vast-admin/coordinator-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const W = here + 'worker.js';
const MUTANTS = [
  ['common_name not checked', 'identity.common_name === coordinatorId', 'true'],
  ['email absence not checked', 'identity.email === undefined &&', ''],
  ['sub not checked', "identity.sub === '' &&", ''],
  ['type app not checked', "identity.type === 'app' &&", ''],
  [
    'empty var guard removed',
    "typeof coordinatorId === 'string' &&\n      /^[a-f0-9]{32}\\.access$/.test(coordinatorId) &&",
    'identity.common_name !== undefined &&',
  ],
  ['client id format not checked', '/^[a-f0-9]{32}\\.access$/.test(coordinatorId)', 'true'],
  ['route allowlist removed', "kind === 'coordinator' &&\n      !(", 'false &&\n      !('],
  [
    'hosts routes allowed',
    "['/', '/api/list', '/log'].includes(url.pathname)",
    "['/', '/api/list', '/log'].includes(url.pathname) || url.pathname.startsWith('/api/hosts')",
  ],
  [
    'method not checked on allowlist',
    "(request.method === 'POST' &&\n          [",
    '(true &&\n          [',
  ],
  ['owner kind loses its marker', "kind = 'owner'", "kind = 'coordinator'"],
];
const selected = process.argv.slice(2);
const original = readFileSync(W, 'utf8');
let failed = false;
try {
  for (const [name, before, after] of MUTANTS) {
    if (selected.length && !selected.includes(name)) continue;
    if (original.split(before).length !== 2) {
      console.log(
        JSON.stringify({
          mutant: name,
          ok: false,
          why: 'ambiguous or missing',
        }),
      );
      failed = true;
      continue;
    }
    writeFileSync(W, original.split(before).join(after));
    const run = spawnSync(
      process.execPath,
      [
        '--no-warnings',
        '--test',
        '--test-reporter=tap',
        '--test-timeout=60000',
        here + 'test/coordinator.test.mjs',
      ],
      { encoding: 'utf8', timeout: 300000, maxBuffer: 64 * 1024 * 1024 },
    );
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)].map((m) => m[1]);
    const complete = /^# tests \d+$/m.test(run.stdout ?? '');
    const ok = run.status !== 0 && kills.length >= 2 && complete;
    failed ||= !ok;
    console.log(
      JSON.stringify({
        mutant: name,
        kills: kills.length,
        complete,
        ok,
        first: kills.slice(0, 2),
      }),
    );
  }
} finally {
  writeFileSync(W, original);
}
if (failed) {
  console.log('SURVIVING OR UNDER-KILLED MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
