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
    'const counts = (p, n) => !p.dirOnly || n < depth || directory;',
    'const counts = () => true;',
  ],
  [
    'basename anchoring',
    'src/treekey.js',
    '    if (p.anchored) {\n      const reach',
    '    if (true) {\n      const reach',
  ],
  [
    'parent directories',
    'src/treekey.js',
    'for (let n = 1; n <= depth; n++) if (reach[n] && counts(p, n)) return true;',
    'if (reach[depth] && counts(p, depth)) return true;',
  ],
  [
    'trailing ** matches nothing below',
    'src/treekey.js',
    '        if (last) {\n          if (seen) next[j] = 1;',
    '        if (false) {\n          if (seen) next[j] = 1;',
  ],
  // N1: deterministic bounds (depth, '**' count, step counter).
  [
    'no depth cap',
    'src/treekey.js',
    "  if (depth > LIMITS.depth) throw new LimitError('path depth');\n",
    '',
  ],
  [
    'no ** cap',
    'src/treekey.js',
    "  if (parts.filter((s) => s === '**').length > LIMITS.anyPerPattern) return null;\n",
    '',
  ],
  [
    'no step budget',
    'src/treekey.js',
    "    if (this.left < 0) throw new LimitError('matcher steps');\n",
    '',
  ],
  // H1: a backtracking matcher (the RegExp of the first version) must be caught by the timing test.
  [
    'backtracking glob',
    'src/treekey.js',
    'export function globSegment(pattern, text, budget = null) {',
    "export function globSegment(pattern, text, budget = null) {\n  const d = (b) => new TextDecoder().decode(b);\n  const source = d(pattern).replace(/[.+]/g, '\\\\$&').replaceAll('*', '[^/]*').replaceAll('?', '[^/]');\n  return new RegExp('^' + source + '$').test(d(text));",
  ],
  // H2: protected roots, the ignore set in the key, empty listings and empty trees.
  [
    'code roots ignorable',
    'src/treekey.js',
    '  for (const [path, dir] of PROTECTED) if (ignored(patterns, path, dir)) return null;\n',
    '',
  ],
  [
    'ignore set not in key',
    'src/treekey.js',
    "let text = TREE_KEY_VERSION + '\\nignore ' + ignoreDigest + '\\n';",
    "let text = TREE_KEY_VERSION + '\\n';",
  ],
  ['empty tree keyed', 'src/treekey.js', '  if (!kept.length) return null;\n', ''],
  // (No mutant for the empty root-listing check: an empty listing also yields no kept file, so
  // treeKeyOf returns null either way; the check stays as defence in depth.)
  [
    'replacement character',
    'src/treekey.js',
    '!/[\\0\\n\\ufffd]/.test(path)',
    '!/[\\0\\n]/.test(path)',
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
    'carry-over on by default',
    'src/results.js',
    "const carry = env.TREE_KEY_CARRY_OVER === 'true';",
    "const carry = env.TREE_KEY_CARRY_OVER !== 'false';",
  ],
  [
    'claimed tree trusted',
    'src/results.js',
    "  if (treeKind === 'tree' && !verifiedTree) treeKind = 'claimed';\n",
    '',
  ],
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
  [
    'tampered row skipped',
    'src/requirements.js',
    "      missing.push({ family: r.family, die: r.die, platform: r.platform, reason: 'TAMPERED' });",
    '      void r;',
  ],
  [
    'tagged in-flight row trusted',
    'src/requirements.js',
    'if (r.tampered === true || (r.verdict !== null && r.valid !== true))',
    'if (r.verdict !== null && r.valid !== true)',
  ],
  [
    'tagged in-flight row re-signed',
    'src/results.js',
    'WHERE r.verdict IS NULL AND r.tag IS NULL',
    'WHERE r.verdict IS NULL',
  ],
  [
    'untagged order',
    'src/requirements.js',
    'a.ended_at < b.ended_at ? 1 : a.ended_at > b.ended_at ? -1',
    'a.sort_at < b.sort_at ? 1 : a.sort_at > b.sort_at ? -1',
  ],
  [
    'passes not a streak',
    'src/requirements.js',
    "          answers[streak].verdict === 'PASS' &&",
    "          answers.slice(streak).some((x) => x.verdict === 'PASS') &&",
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
    'caller picks the base',
    'src/ingest.js',
    "Object.keys(body).sort().join(',') !== 'commit_sha,tier' ||",
    "!['commit_sha,tier', 'base,commit_sha,tier'].includes(Object.keys(body).sort().join(',')) ||",
  ],
  [
    'base not from tier',
    'src/ingest.js',
    "(normalTier(tier) === 'small' ? 'next' : 'master')",
    "'next'",
  ],
  [
    'tree attempt saved late',
    'src/ingest.js',
    "  await ledger.put(TREE_PREFIX + sha, { kind: 'pending', attempts, at: now });\n",
    '',
  ],
  ['store attempt saved late', 'src/ingest.js', '    await ledger.put(key, entry);\n', ''],
  [
    'no tick budget',
    'src/ingest.js',
    'if (worked >= PER_TICK || clock() >= deadline) break;',
    'if (worked >= PER_TICK) break;',
  ],
  [
    'no stale closing',
    'src/ingest.js',
    '    if (runId) await closeStale(env, { now, runId, olderThanMs: 0, limit: 1 });\n',
    '',
  ],
  [
    'store before expiry',
    'src/index.js',
    "          await ledger.expire(now(), cfg);\n          // After cleanup and expiry, bounded in time (src/ingest.js); tree keys come from the\n          // cache that storeTrees fills outside this queue.\n          if (storeEnabled(env))\n            try {\n              await storeTick({ ledger, env, now: now() });\n            } catch {\n              console.log(JSON.stringify({ event: 'store', state: 'TICK_FAILED' }));\n            }\n",
    "          if (storeEnabled(env))\n            try {\n              await storeTick({ ledger, env, now: now() });\n            } catch {\n              console.log(JSON.stringify({ event: 'store', state: 'TICK_FAILED' }));\n            }\n          await ledger.expire(now(), cfg);\n",
  ],
  [
    'check inside the queue',
    'src/index.js',
    '        if (path === CHECK_PATH) return await perform(request);\n',
    '',
  ],
  [
    'shared admission counter',
    'src/index.js',
    'if (++checkCount > DEFAULTS.admissionPerMinute)',
    'if (++admissionCount > DEFAULTS.admissionPerMinute)',
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
  // Integration of #258 with #259 (review rev259 F7, 2026-11-05): tier and hardware of a row.
  ['release tier ignored', 'src/ingest.js', "  if (profile === 'release') return 'largest';\n", ''],
  [
    'record tier ignored',
    'src/ingest.js',
    '    tier: storeTierOfRecord(r),',
    '    tier: storeTierOf(budgetClass(r)),',
  ],
  [
    'profile instead of budget class',
    'src/ingest.js',
    '  const profile = budgetClass(r);',
    "  const profile = r.profile ?? 'check';",
  ],
  [
    'wrong die passes',
    'src/ingest.js',
    "        'MATCH')\n  )\n    verdict = 'UNATTRIBUTED';",
    "        'UNKNOWN')\n  )\n    verdict = 'UNATTRIBUTED';",
  ],
  [
    'family not derived',
    'src/ingest.js',
    'family: r.family ?? cfgFamilyOf(r.die, cfg)',
    'family: r.family',
  ],
  // Review of PR #269, R2 (2026-11-05): Windows gate runs of the largest tier.
  [
    'windows runs not counted',
    'src/requirements.js',
    "if (platform === 'windows' && block.min_windows_runs !== null)",
    'if (false)',
  ],
  ['windows runs across a FAIL', 'src/requirements.js', 'answers.slice(0, streak)', 'answers'],
  [
    'second windows run not counted',
    'src/requirements.js',
    "Object.freeze(['windows_gate', 'windows_gate_2'])",
    "Object.freeze(['windows_gate'])",
  ],
  [
    'min_windows_runs without windows',
    'src/requirements.js',
    "    fail('min_windows_runs needs platform windows');\n",
    '    ;\n',
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
    writeFileSync(path, original.split(before).join(after));
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
