// Mutation check for the key invariants: read-only, fixed SQL, parameter validation, no leaks.
// Each mutant is applied to a temporary copy (the checkout is never edited) and must make at
// least one non-fixture test fail. Run: npm run mutate [-- name ...]
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('..', import.meta.url));
const fixture = fileURLToPath(new URL('../../../site/status/fixture.json', import.meta.url));

const MUTANTS = [
  // read-only / method boundary
  ['method guard', 'src/api.js', "if (request.method !== 'GET') return", 'if (false) return'],
  [
    'CORS on 405',
    'src/http.js',
    "      Allow: 'GET',",
    "      Allow: 'GET',\n      'Access-Control-Allow-Origin': '*',",
  ],
  [
    'write statement',
    'src/queries.js',
    'health: `SELECT 1 AS ok`',
    'health: `UPDATE audit SET actor = actor RETURNING 1 AS ok`',
  ],
  [
    'second statement',
    'src/queries.js',
    'runLogs: `SELECT r.logs_json FROM runs r WHERE r.id = ?1`',
    'runLogs: `SELECT r.logs_json FROM runs r WHERE r.id = ?1; DELETE FROM audit`',
  ],
  // fixed SQL
  [
    'raw SQL method',
    'src/db.js',
    '  async first(name, params = []) {',
    '  async raw(sql) {\n    return this.d1.prepare(sql).all();\n  }\n\n  async first(name, params = []) {',
  ],
  [
    'SQL concatenation',
    'src/queries.js',
    'dieById: `SELECT d.id AS die, d.name AS die_name, d.family_id FROM dies d WHERE d.id = ?1`,',
    "dieById: 'SELECT d.id AS die, d.name AS die_name, d.family_id FROM dies d WHERE d.id = ' + '?1',",
  ],
  ['statement by text', 'src/db.js', 'if (!Object.hasOwn(QUERIES, name))', 'if (false)'],
  [
    'tag selected',
    'src/queries.js',
    'r.finished_at, r.sort_at, r.source, r.tier,\n           p.host_driver',
    'r.finished_at, r.sort_at, r.source, r.tier, r.tag,\n           p.host_driver',
  ],
  // parameter validation
  [
    'unknown parameter',
    'src/validate.js',
    'if (!Object.hasOwn(allowed, name)) throw',
    'if (false) throw',
  ],
  ['repeated parameter', 'src/validate.js', 'if (seen.has(name)) throw', 'if (false) throw'],
  [
    'driver pattern',
    'src/validate.js',
    'driver: /^[0-9]{3}\\.[0-9]{1,3}(?:\\.[0-9]{1,3})?$/',
    'driver: /./',
  ],
  [
    'die pattern',
    'src/validate.js',
    'die: /^[A-Z]{2}[0-9]{3}[A-Z]?$/',
    "die: /^[A-Za-z0-9%' =-]+$/",
  ],
  ['limit ceiling', 'src/validate.js', 'n < 1 || n > MAX_LIMIT', 'n < 1'],
  ['cursor fields', 'src/validate.js', '!PATTERNS.time.test(parsed.s) ||', ''],
  ['cursor keys', 'src/validate.js', 'Object.keys(parsed).length !== 2 ||', ''],
  [
    'run id path',
    'src/api.js',
    "if (!PATTERNS.runId.test(id)) throw new ApiError(400, 'bad_request', 'invalid run id');\n    const row = await db.first('runById'",
    "const row = await db.first('runById'",
  ],
  // leaks and publication rules
  ['SPEC scores shown', 'src/api.js', 'if (HIDDEN_SCORE_SUITES.has(row.suite))', 'if (false)'],
  ['unlisted log', 'src/api.js', '!logNames(row.logs_json).includes(name)', 'false'],
  ['logs origin', 'src/validate.js', '    !onSite ||\n', ''],
  ['logs scheme', 'src/validate.js', "url.protocol !== 'https:' ||", ''],
  [
    'running immutable',
    'src/api.js',
    'const final = row.verdict !== null && row.finished_at !== null;',
    'const final = true;',
  ],
  [
    'outbound fetch',
    'src/index.js',
    '    return handle(request, env);',
    "    fetch('https://api.github.com/zen').catch(() => {});\n    return handle(request, env);",
  ],
  [
    'error echo',
    'src/api.js',
    "return errorResponse(500, 'internal', 'internal error');",
    "return errorResponse(500, 'internal', String(error));",
  ],
];

const selected = process.argv.slice(2);
let failed = false;
for (const [name, file, before, after] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const root = mkdtempSync(join(tmpdir(), 'kf-platform-mutant-'));
  const copy = join(root, 'ci/platform-api');
  try {
    cpSync(here, copy, { recursive: true, filter: (p) => !p.includes('node_modules') });
    mkdirSync(join(root, 'site/status'), { recursive: true });
    cpSync(fixture, join(root, 'site/status/fixture.json'));
    const target = join(copy, file);
    const original = readFileSync(target, 'utf8');
    if (original.split(before).length !== 2) throw Error('ambiguous or missing mutation: ' + name);
    writeFileSync(target, original.replace(before, after));
    const tests = ['api', 'static', 'row-tag', 'fixture'].map((t) =>
      join(copy, 'test', t + '.test.js'),
    );
    const run = spawnSync(process.execPath, ['--no-warnings', '--test', ...tests], {
      cwd: copy,
      encoding: 'utf8',
      timeout: 180000,
      maxBuffer: 16 * 1024 * 1024,
    });
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)]
      .map((m) => m[1])
      .filter((t) => !/fixture/.test(t));
    const ok = run.status !== 0 && kills.length >= 1 && !run.error;
    failed ||= !ok;
    console.log(JSON.stringify({ mutant: name, status: run.status, kills, ok }));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}
if (failed) {
  console.log('SURVIVING MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
