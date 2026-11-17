// Mutation check for the coordinator identity and the JSON content-type rule (worker.js), GPU-free.
// Each mutant is one exact-once replacement applied to a COPY of the admin tree in a temp directory
// (the real worker.js is never written, so a killed run leaves nothing behind); test/coordinator.test.mjs
// of the copy runs. Every mutant must make at least TWO named tests fail:
//   node ci/vast-admin/coordinator-mutants.mjs [name ...]
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const MUTANTS = [
  [
    'content type not required on bodyless writes',
    'if (mutating && !isJson()) return',
    'if (false) return',
  ],
  [
    'plan and start accept any content type',
    "if (!isJson()) return json({ error: 'Unsupported media type' }, 415);\n      try {\n        const reader = request.body?.getReader();\n        if (!reader) throw Error();\n        const chunks = [];\n        let size = 0;\n        try {\n          while (true) {\n            const { value, done } = await reader.read();\n            if (done) break;\n            size += value.byteLength;\n            if (size > 512)",
    'try {\n        const reader = request.body?.getReader();\n        if (!reader) throw Error();\n        const chunks = [];\n        let size = 0;\n        try {\n          while (true) {\n            const { value, done } = await reader.read();\n            if (done) break;\n            size += value.byteLength;\n            if (size > 512)',
  ],
  [
    'content type check accepts any type',
    "toLowerCase() ===\n      'application/json'",
    "toLowerCase() !==\n      'application/json'",
  ],
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
  [
    'any email-less app JWT is the coordinator',
    'identity.email === undefined &&\n      identity.type',
    'identity.email === undefined ||\n      identity.type',
  ],
  [
    'coordinator kind refused on hosts routes',
    "headers['X-Kf-Identity'] = kind;",
    "headers['X-Kf-Identity'] = kind;\n    if (kind === 'coordinator' && /^\\/api\\/hosts/.test(new URL(request.url).pathname)) return json({ error: 'Forbidden' }, 403);",
  ],
  ['owner kind loses its marker', "kind = 'owner'", "kind = 'coordinator'"],
];
const selected = process.argv.slice(2);
const root = mkdtempSync(join(tmpdir(), 'kf-admin-mutants-'));
const copy = join(root, 'ci', 'vast-admin');
mkdirSync(copy, { recursive: true });
for (const f of ['worker.js', 'package.json']) cpSync(here + f, join(copy, f));
cpSync(here + 'test', join(copy, 'test'), { recursive: true });
symlinkSync(here + 'node_modules', join(copy, 'node_modules'));
symlinkSync(join(here, '..', 'vast-broker'), join(root, 'ci', 'vast-broker'));
const W = join(copy, 'worker.js');
const original = readFileSync(W, 'utf8');
let failed = false;
try {
  for (const [name, before, after] of MUTANTS) {
    if (selected.length && !selected.includes(name)) continue;
    if (original.split(before).length !== 2) {
      console.log(JSON.stringify({ mutant: name, ok: false, why: 'ambiguous or missing' }));
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
        join(copy, 'test', 'coordinator.test.mjs'),
      ],
      { encoding: 'utf8', timeout: 300000, maxBuffer: 64 * 1024 * 1024 },
    );
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)].map((m) => m[1]);
    const complete = /^# tests \d+$/m.test(run.stdout ?? '');
    const ok = run.status !== 0 && kills.length >= 2 && complete;
    failed ||= !ok;
    console.log(
      JSON.stringify({ mutant: name, kills: kills.length, complete, ok, first: kills.slice(0, 2) }),
    );
  }
} finally {
  rmSync(root, { recursive: true, force: true });
}
if (failed) {
  console.log('SURVIVING OR UNDER-KILLED MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
