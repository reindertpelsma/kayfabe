// Mutation check for the result store, the tree key and the requirements check (GPU-free).
// Each mutant is applied in place, the store tests run, and the file is restored before the
// next mutant (also on error). Every mutant must make at least one named test fail.
// Run from anywhere, with no other test or build running in this checkout:
//   node ci/vast-broker/store-mutants.mjs [name ...]
import { readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('.', import.meta.url));
const TESTS = [
  'test/treekey.test.js',
  'test/treekey-parity.test.js',
  'test/requirements.test.js',
  'test/results-store.test.js',
  'test/store-ingest.test.js',
].map((t) => here + t);

const MUTANTS = [
  // tree key and .workerignore matcher
  [
    'negation accepted',
    'src/treekey.js',
    'const ALLOWED = /^[A-Za-z0-9._+@/*?-]+$/;',
    'const ALLOWED = /^[A-Za-z0-9._+@/*?!-]+$/;',
  ],
  [
    'directory-only ignored',
    'src/treekey.js',
    'if ((!p.dirOnly || isDir) && matches(p, candidate))',
    'if (matches(p, candidate))',
  ],
  [
    'basename anchoring',
    'src/treekey.js',
    "return p.regex.test(p.anchored ? path : path.slice(path.lastIndexOf('/') + 1));",
    'return p.regex.test(path);',
  ],
  [
    'parent directories',
    'src/treekey.js',
    'for (let i = 1; i <= parts.length; i++) {',
    'for (let i = parts.length; i <= parts.length; i++) {',
  ],
  [
    'truncated listing',
    'src/treekey.js',
    'if (!listing || listing.truncated !== false || !Array.isArray(listing.tree)) return null;',
    'if (!listing || !Array.isArray(listing.tree)) return null;',
  ],
  [
    'byte order',
    'src/treekey.js',
    'kept.sort((a, b) => compareBytes(a.path, b.path));',
    'kept.sort((a, b) => (a.path < b.path ? -1 : 1));',
  ],
  [
    'mode in key',
    'src/treekey.js',
    "text += e.path + '\\0' + e.mode + '\\0' + e.sha + '\\n';",
    "text += e.path + '\\0' + e.sha + '\\n';",
  ],
  [
    'unknown ignore entry',
    'src/treekey.js',
    "      if (ignoreEntry.type !== 'blob' || !['100644', '100755'].includes(ignoreEntry.mode))\n        return null;\n",
    '',
  ],
  // store
  ['final overwritten', 'src/results.js', 'if (before && before.verdict !== null)', 'if (false)'],
  [
    'untagged counts',
    'src/results.js',
    '(await verifyRunTag(env.ROW_TAG_KEY, row, own, row.tag));',
    'true;',
  ],
  [
    'token-shaped stored',
    'src/results.js',
    "if (typeof v === 'string' && TOKEN_SHAPED.test(v)) bad('token-shaped');",
    '',
  ],
  ['stages untagged', 'src/results.js', '  parts.push(await hex(lines));\n', ''],
  [
    'no tag key needed',
    'src/results.js',
    "if (!validTagKey(env.ROW_TAG_KEY)) return result(false, 'NO_TAG_KEY');",
    '',
  ],
  [
    'redaction tail',
    'src/results.js',
    'let cut = Math.max(0, text.length - this.hold);',
    'let cut = text.length;',
  ],
  // requirements
  [
    'retry answers',
    'src/requirements.js',
    "(r.verdict === 'PASS' || r.verdict === 'FAIL')",
    '(r.verdict !== null)',
  ],
  [
    'FAIL not cached',
    'src/requirements.js',
    "        if (answers[0].verdict === 'FAIL') return 'FAIL';\n",
    '',
  ],
  ['tier rank', 'src/requirements.js', 'TIER_RANK[r.tier] >= TIER_RANK[block.tier] &&', ''],
  [
    'stale in-flight',
    'src/requirements.js',
    'Date.parse(r.started_at) > now - INFLIGHT_MAX_MS',
    'true',
  ],
  [
    'min dies',
    'src/requirements.js',
    'if (ok >= block.min_dies) continue;',
    'if (ok >= 1) continue;',
  ],
  [
    'platform coverage',
    'src/requirements.js',
    "const covers = (row, platform) => row.platform === platform || row.platform === 'both';",
    'const covers = () => true;',
  ],
  [
    'empty requirements met',
    'src/requirements.js',
    '  if (!wanted.length)\n    return {\n      met: false,',
    '  if (!wanted.length)\n    return {\n      met: true,',
  ],
  [
    'unknown key accepted',
    'src/requirements.js',
    "if (!KEYS.has(key)) throw new RequirementsError(line, 'unknown key ' + key);",
    '',
  ],
  // ingest and the check route
  [
    'non-result PASS',
    'src/ingest.js',
    "if (r.verdict === 'PASS' && r.result?.verdict === 'PASS' && r.resultAt !== undefined)",
    "if (r.verdict === 'PASS')",
  ],
  ['unreported FAIL', 'src/ingest.js', '    r.firstReportAt !== undefined &&\n', ''],
  [
    'store without tag key',
    'src/ingest.js',
    "typeof env.ROW_TAG_KEY === 'string' &&\n  env.ROW_TAG_KEY.length >= 32",
    'true',
  ],
  [
    'any base branch',
    'src/ingest.js',
    "!['master', 'next'].includes(body.base) ||",
    "typeof body.base !== 'string' ||",
  ],
  [
    'replacement not frozen',
    'src/index.js',
    '        if (retrySource) await freeze(ledger, retrySource);\n',
    '',
  ],
  [
    'check without authorization',
    'src/index.js',
    '    if (path === CHECK_PATH) {\n      await github.authorize(c);',
    '    if (path === CHECK_PATH) {',
  ],
];

const selected = process.argv.slice(2);
let failed = false;
for (const [name, file, before, after] of MUTANTS) {
  if (selected.length && !selected.includes(name)) continue;
  const path = here + file;
  const original = readFileSync(path, 'utf8');
  if (original.split(before).length !== 2) {
    console.log(JSON.stringify({ mutant: name, ok: false, why: 'ambiguous or missing' }));
    failed = true;
    continue;
  }
  try {
    writeFileSync(path, original.replace(before, after));
    const run = spawnSync(process.execPath, ['--no-warnings', '--test', ...TESTS], {
      cwd: here,
      encoding: 'utf8',
      timeout: 300000,
      maxBuffer: 64 * 1024 * 1024,
    });
    const kills = [...(run.stdout ?? '').matchAll(/^not ok \d+ - (.+)$/gm)]
      .map((m) => m[1])
      .filter((t) => !t.endsWith('.test.js'));
    const ok = run.status !== 0 && kills.length >= 1 && !run.error;
    failed ||= !ok;
    console.log(JSON.stringify({ mutant: name, status: run.status, kills, ok }));
  } finally {
    writeFileSync(path, original);
  }
}
if (failed) {
  console.log('SURVIVING MUTANT(S): see ok=false above');
  process.exitCode = 1;
}
