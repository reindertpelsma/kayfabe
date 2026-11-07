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
const names = fileURLToPath(new URL('../../../site/status/gpu_names.json', import.meta.url));
const defaults = fileURLToPath(new URL('../../vast-broker/defaults.json', import.meta.url));

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
    'if (!Object.hasOwn(allowed, name))\n      throw',
    'if (false)\n      throw',
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
  // CI Worker result store (migration 0002)
  [
    'ci tag selected',
    'src/queries.js',
    'r.platform, r.verdict, r.started_at, r.ended_at, r.log_key, r.source,',
    'r.platform, r.verdict, r.started_at, r.ended_at, r.log_key, r.source, r.tag,',
  ],
  [
    'ci private joined',
    'src/queries.js',
    'FROM ci_runs r LEFT JOIN ci_host_facts f ON f.run_id = r.run_id',
    'FROM ci_runs r LEFT JOIN ci_host_facts f ON f.run_id = r.run_id LEFT JOIN ci_runs_private p ON p.run_id = r.run_id',
  ],
  ['ci tree pattern', 'src/validate.js', 'hash: /^[0-9a-f]{64}$/,', 'hash: /^[0-9A-Za-z]{1,64}$/,'],
  [
    'ci run id path',
    'src/api.js',
    "if (!PATTERNS.runId.test(id)) throw new ApiError(400, 'bad_request', 'invalid run id');\n    const row = await db.first('ciRunById'",
    "const row = await db.first('ciRunById'",
  ],
  [
    'ci newest cell',
    'src/queries.js',
    'PARTITION BY r.tier, r.family, r.die, r.platform\n               ORDER BY r.sort_at DESC, r.run_id DESC\n             ) AS rn\n      FROM ci_runs r\n      WHERE (?1',
    'PARTITION BY r.tier, r.family, r.die, r.platform\n               ORDER BY r.sort_at ASC, r.run_id DESC\n             ) AS rn\n      FROM ci_runs r\n      WHERE (?1',
  ],
  // Setup picker (migration 0003): /v1/ci/setups and /v1/ci/facets
  [
    'setups windows hang charged to Linux only',
    'src/queries.js',
    "WHEN 'windows' THEN s.name NOT IN ('kvm_tests', 'gates',",
    "WHEN 'windows' THEN s.name IN ('windows_gate', 'windows_retry_time', 'specviewperf') AND s.name NOT IN ('kvm_tests', 'gates',",
  ],
  [
    'setups windows stage rows charged to Linux',
    'src/queries.js',
    "ELSE s.name NOT LIKE 'windows%' AND s.name <> 'specviewperf'",
    "ELSE s.name NOT LIKE 'windows%'",
  ],
  [
    'setups linux stage rows charged to Windows',
    'src/queries.js',
    "'thin_suite', 'birth_census',",
    "'thin_suite',",
  ],
  [
    'setups running run counted',
    'src/queries.js',
    '      WHERE r.verdict IS NOT NULL\n        AND (?1 IS NULL OR o.os = ?1)',
    '      WHERE 1\n        AND (?1 IS NULL OR o.os = ?1)',
  ],
  [
    'setups both-run counted once',
    'src/queries.js',
    "ON r.platform = o.os OR r.platform = 'both'",
    'ON r.platform = o.os',
  ],
  [
    'setups last pass of any result',
    'src/queries.js',
    "MAX(CASE WHEN x.res = 'PASS' AND x.rn_result = 1 THEN x.run_id END) AS pass_id",
    'MAX(CASE WHEN x.rn_result = 1 THEN x.run_id END) AS pass_id',
  ],
  [
    'setups cpu filter matches unreported',
    'src/queries.js',
    'AND (?6 IS NULL OR c.cpu_vendor = ?6)',
    'AND (?6 IS NULL OR c.cpu_vendor = ?6 OR c.cpu_vendor IS NULL)',
  ],
  [
    'setups driver filter host only',
    'src/queries.js',
    'AND (?7 IS NULL OR c.host_driver = ?7 OR c.guest_driver = ?7)',
    'AND (?7 IS NULL OR c.host_driver = ?7)',
  ],
  [
    'setups tier filter ignored',
    'src/queries.js',
    'AND (?8 IS NULL OR r.tier = ?8)\n        AND ((?9',
    'AND (?8 IS NULL OR 1)\n        AND ((?9',
  ],
  [
    'setups private table joined',
    'src/queries.js',
    'LEFT JOIN ci_host_facts f ON f.run_id = r.run_id\n      WHERE r.verdict IS NOT NULL\n        AND (?1',
    'LEFT JOIN ci_host_facts f ON f.run_id = r.run_id\n      LEFT JOIN ci_runs_private pr ON pr.run_id = r.run_id\n      WHERE r.verdict IS NOT NULL\n        AND (?1',
  ],
  [
    'setups row tag selected',
    'src/queries.js',
    '             r.sort_at, o.os AS os, r.source AS source,\n             CASE',
    '             r.sort_at, o.os AS os, r.source AS source, r.tag AS tag,\n             CASE',
  ],
  [
    'setups truncation not reported',
    'src/api.js',
    "limit + 1,\n    ];\n    const rows = await db.all('ciSetups'",
    "limit,\n    ];\n    const rows = await db.all('ciSetups'",
  ],
  [
    'setups claims unmarked',
    'src/api.js',
    "['cpu_vendor', 'host_driver', 'guest_driver']);",
    '[]);',
  ],
  ['setups other count', 'src/api.js', 'other: r.runs - r.passed - r.failed,', 'other: 0,'],
  [
    'setups gpu list pattern',
    'src/validate.js',
    'gpuList: /^[A-Z]{2}[0-9]{3}[A-Z]?(?:,[A-Z]{2}[0-9]{3}[A-Z]?){0,3}$/,',
    "gpuList: /^[A-Za-z0-9%' ,=-]+$/,",
  ],
  ['setups os pattern', 'src/validate.js', 'os: /^(?:linux|windows)$/,', 'os: /./,'],
  [
    'setups cpu pattern',
    'src/validate.js',
    'cpuVendor: /^(?:amd|intel|arm|other)$/,',
    'cpuVendor: /./,',
  ],
  [
    'setups community always shown',
    'src/queries.js',
    "OR (?10 = 1 AND r.source IN ('reported', 'reviewed')))",
    "OR (r.source IN ('reported', 'reviewed')))",
  ],
  [
    'setups source flag ignored',
    'src/queries.js',
    "AND ((?9 = 1 AND r.source = 'reproduced')",
    "AND ((r.source = 'reproduced')",
  ],
  ['setups report tier ignored', 'src/queries.js', 'AND (?11 IS NULL OR (?11', 'AND (1 OR (?11'],
  [
    'setups report tier swapped',
    'src/queries.js',
    "(?11 = 'unverified' AND r.source = 'reported')",
    "(?11 = 'unverified' AND r.source = 'reviewed')",
  ],
  [
    'setups sources merged in ranking',
    'src/queries.js',
    'PARTITION BY p.os, p.die, p.source, p.cpu_vendor, p.host_driver, p.guest_driver\n               ORDER BY',
    'PARTITION BY p.os, p.die, p.cpu_vendor, p.host_driver, p.guest_driver\n               ORDER BY',
  ],
  ['setups community default on', 'src/api.js', "sources.has('community') ? 1 : 0,", '1,'],
  [
    'setups raw source vocabulary',
    'src/api.js',
    "v === 'reproduced' ? 'reproduced' : v === 'reported' || v === 'reviewed' ? 'community' : null;",
    'v;',
  ],
  [
    'setups source pattern',
    'src/validate.js',
    '  sourceList: /^(?:reproduced|imported|community)(?:,(?:reproduced|imported|community)){0,2}$/,',
    '  sourceList: /./,',
  ],
  [
    'setups report tier pattern',
    'src/validate.js',
    'reportTier: /^(?:unverified|reviewed)$/,',
    'reportTier: /./,',
  ],
  [
    'facets source always available',
    'src/api.js',
    "available: count(['reproduced']) > 0 },",
    'available: true },',
  ],
  [
    'facets running run counted',
    'src/queries.js',
    'FROM ci_runs r WHERE r.verdict IS NOT NULL\n    GROUP BY r.die ORDER BY family, r.die LIMIT 100',
    'FROM ci_runs r\n    GROUP BY r.die ORDER BY family, r.die LIMIT 100',
  ],
  [
    'facets claim of unfinished run',
    'src/queries.js',
    'FROM ci_run_claims c JOIN ci_runs r ON r.run_id = c.run_id\n    WHERE r.verdict IS NOT NULL\n    GROUP BY value',
    'FROM ci_run_claims c JOIN ci_runs r ON r.run_id = c.run_id\n    GROUP BY value',
  ],
  [
    'facets unchecked driver output',
    'src/api.js',
    "const driverOut = (v) => (typeof v === 'string' && DRIVER_OUT.test(v) ? v : null);",
    'const driverOut = (v) => v ?? null;',
  ],
  // Run reports, subruns and stats (migration 0004): ranking, parameters, bounds
  [
    'rank ladder commit/cpu swapped',
    'src/rank.js',
    "  'commit',\n  'cpu',\n  'host_driver',",
    "  'cpu',\n  'commit',\n  'host_driver',",
  ],
  [
    'rank relaxed count ignored',
    'src/rank.js',
    'const closeness = [relaxed.length, worst, ...vector];',
    'const closeness = [worst, ...vector];',
  ],
  [
    'rank ladder position ignored',
    'src/rank.js',
    'const closeness = [relaxed.length, worst, ...vector];',
    'const closeness = [relaxed.length, 0, ...vector];',
  ],
  [
    'rank closeness vector ignored',
    'src/rank.js',
    'const closeness = [relaxed.length, worst, ...vector];',
    'const closeness = [relaxed.length, worst];',
  ],
  [
    'rank tie-break not total',
    'src/rank.js',
    'const tail = [-(Number.isFinite(ended) ? ended : 0), row.run_id, row.sub];',
    'const tail = [-(Number.isFinite(ended) ? ended : 0)];',
  ],
  [
    'rank recency inverted',
    'src/rank.js',
    'const tail = [-(Number.isFinite(ended) ? ended : 0), row.run_id, row.sub];',
    'const tail = [Number.isFinite(ended) ? ended : 0, row.run_id, row.sub];',
  ],
  [
    'rank other family not relaxed',
    'src/rank.js',
    "put('family', crossFamily ? { level: 1, dist: [0, 0, 0] } : null, wantedFamily, candFamily);",
    "put('family', null, wantedFamily, candFamily);",
  ],
  [
    'rank relax set not enforced',
    'src/rank.js',
    'if (relaxed.length > maxRelaxed || relaxed.some((r) => !position.has(r.dim))) continue;',
    'if (relaxed.length > maxRelaxed) continue;',
  ],
  ['rank min_closeness ignored', 'src/rank.js', 'if (relaxed.length > maxRelaxed ||', 'if ('],
  [
    'rank cpu architecture level',
    'src/rank.js',
    'c.cpu_arch !== null && c.cpu_arch === wanted.cpu.arch',
    'false',
  ],
  [
    'rank driver distance',
    'src/rank.js',
    '      Math.abs(want[0] - found[0]),\n      Math.abs(want[1] - found[1]),\n      Math.abs(want[2] - found[2]),',
    '      0,\n      0,\n      0,',
  ],
  [
    'rank same tree not exact',
    'src/rank.js',
    '(wanted.tree && c.tree_hash === wanted.tree) ||',
    '(false) ||',
  ],
  [
    'rank unknown reference',
    'src/rank.js',
    'const ref = wanted.refTime ?? ctx.newest;',
    'const ref = wanted.refTime ?? 0;',
  ],
  ['rank same-day window', 'src/rank.js', 'gap <= DAY_MS;', 'false;'],
  ['rank duration sort', 'src/rank.js', "sort === 'duration'", 'false'],
  ['rank recent sort', 'src/rank.js', "sort === 'recent'", 'false'],
  [
    'dies order abs',
    'src/dies.js',
    'return i < 0 || j < 0 ? null : Math.abs(i - j);',
    'return i < 0 || j < 0 ? null : i - j;',
  ],
  [
    'dies turing order',
    'src/dies.js',
    "turing: Object.freeze(['TU117', 'TU116', 'TU106', 'TU104', 'TU102']),",
    "turing: Object.freeze(['TU116', 'TU117', 'TU106', 'TU104', 'TU102']),",
  ],
  [
    'subruns require relaxable',
    'src/subruns.js',
    'const relax = (given ?? [...DIMS]).filter((d) => !require.includes(d) && !ignore.includes(d));',
    'const relax = (given ?? [...DIMS]).filter((d) => !ignore.includes(d));',
  ],
  [
    'subruns ignore matched',
    'src/subruns.js',
    'const relax = (given ?? [...DIMS]).filter((d) => !require.includes(d) && !ignore.includes(d));',
    'const relax = (given ?? [...DIMS]).filter((d) => !require.includes(d));',
  ],
  [
    'subruns unknown dimension',
    'src/subruns.js',
    'const unknown = words.some((w) => !DIMS.includes(w));',
    'const unknown = false;',
  ],
  [
    'subruns repeated dimension',
    'src/subruns.js',
    "if (new Set(words).size !== words.length)\n    throw new ApiError(400, 'bad_request', 'repeated dimension'",
    "if (false)\n    throw new ApiError(400, 'bad_request', 'repeated dimension'",
  ],
  [
    'subruns require ignore conflict',
    'src/subruns.js',
    'if (require.some((d) => ignore.includes(d)))',
    'if (false)',
  ],
  [
    'subruns required without value',
    'src/subruns.js',
    'if (!hasWanted(dim, wanted))\n      throw',
    'if (false)\n      throw',
  ],
  ['subruns limit ceiling', 'src/subruns.js', 'n < 1 || n > MAX_SUBRUN_LIMIT', 'n < 1'],
  [
    'subruns include word',
    'src/subruns.js',
    'if (list.some((w) => !allowed.includes(w)))',
    'if (false)',
  ],
  ['subruns cursor of another request', 'src/subruns.js', 'parsed.f !== fp', 'false'],
  [
    'subruns cursor ignored',
    'src/subruns.js',
    'start = at >= 0 ? at + 1 : Math.min(cursor.i, ranked.length);',
    'start = 0;',
  ],
  ['subruns next cursor never', 'src/subruns.js', 'more && last\n        ?', 'false\n        ?'],
  [
    'subruns expansion unbounded',
    'src/subruns.js',
    'const n = cap(TEST_BUDGET, hits.length, 5, 100);',
    'const n = 100000;',
  ],
  [
    'subruns truncation unannounced',
    'src/subruns.js',
    'target[key] = { rows: list.slice(0, cap).map(shape), truncated: list.length > cap };',
    'target[key] = { rows: list.slice(0, cap).map(shape), truncated: false };',
  ],
  [
    'subruns not-run subruns offered',
    'src/queries.js',
    "WHERE r.verdict IS NOT NULL AND s.verdict IN ('PASS', 'FAIL', 'RETRY')\n      AND (?1 IS NULL OR r.die = ?1)",
    'WHERE r.verdict IS NOT NULL\n      AND (?1 IS NULL OR r.die = ?1)',
  ],
  [
    'subruns in-flight sessions offered',
    'src/queries.js',
    "WHERE r.verdict IS NOT NULL AND s.verdict IN ('PASS', 'FAIL', 'RETRY')",
    "WHERE s.verdict IN ('PASS', 'FAIL', 'RETRY')",
  ],
  [
    'subruns die filter ignored',
    'src/queries.js',
    '      AND (?1 IS NULL OR r.die = ?1)\n      AND (?2 IS NULL OR s.os = ?2)',
    '      AND (?1 IS NULL OR 1)\n      AND (?2 IS NULL OR s.os = ?2)',
  ],
  [
    'subruns os filter ignored',
    'src/queries.js',
    '      AND (?2 IS NULL OR s.os = ?2)\n      AND (?3 IS NULL OR s.host_driver = ?3)',
    '      AND (?2 IS NULL OR 1)\n      AND (?3 IS NULL OR s.host_driver = ?3)',
  ],
  [
    'subruns private table joined',
    'src/queries.js',
    '    FROM ci_subruns s\n    JOIN ci_runs r ON r.run_id = s.run_id\n    LEFT JOIN ci_host_facts f ON f.run_id = r.run_id\n    WHERE r.verdict IS NOT NULL AND s.verdict',
    '    FROM ci_subruns s\n    JOIN ci_runs r ON r.run_id = s.run_id\n    LEFT JOIN ci_runs_private pv ON pv.run_id = r.run_id\n    LEFT JOIN ci_host_facts f ON f.run_id = r.run_id\n    WHERE r.verdict IS NOT NULL AND s.verdict',
  ],
  [
    'subruns method guard path',
    'src/api.js',
    "[/^\\/v1\\/ci\\/subruns$/, 'ciSubruns', 'ciSubruns'],",
    "[/^\\/v1\\/ci\\/subruns$/, 'ciSubruns', 'none'],",
  ],
  [
    'stats oldest instead of newest',
    'src/queries.js',
    'PARTITION BY r.die, p.host_driver ORDER BY r.sort_at DESC, r.run_id DESC',
    'PARTITION BY r.die, p.host_driver ORDER BY r.sort_at ASC, r.run_id DESC',
  ],
  [
    'stats groups merged across drivers',
    'src/queries.js',
    'PARTITION BY r.die, p.host_driver ORDER BY',
    'PARTITION BY r.die ORDER BY',
  ],
  [
    'stats os filter ignored',
    'src/queries.js',
    '        AND (?4 IS NULL OR EXISTS (SELECT 1 FROM ci_subruns s\n                                   WHERE s.run_id = r.run_id AND s.os = ?4',
    '        AND (1 OR EXISTS (SELECT 1 FROM ci_subruns s\n                                   WHERE s.run_id = r.run_id AND s.os = ?4',
  ],
  [
    'stats tier filter ignored',
    'src/queries.js',
    '        AND (?2 IS NULL OR r.tier = ?2)\n        AND (?3 IS NULL OR p.host_driver = ?3)',
    '        AND (?2 IS NULL OR 1)\n        AND (?3 IS NULL OR p.host_driver = ?3)',
  ],
  [
    'stats limit ignored',
    'src/queries.js',
    '             l.sort_at DESC, l.run_id DESC\n    LIMIT ?6`',
    '             l.sort_at DESC, l.run_id DESC\n    LIMIT 1000`',
  ],
  [
    'stats perf unbounded',
    'src/subruns.js',
    'rows.slice(0, perfCap).map(perfOut), truncated: rows.length > perfCap',
    'rows.map(perfOut), truncated: false',
  ],
  [
    'stats sort ignored',
    'src/queries.js',
    "ORDER BY CASE ?5 WHEN 'die' THEN l.die WHEN 'driver' THEN l.host_driver END,",
    'ORDER BY',
  ],
  ['report identity from the stored text', 'src/subruns.js', '  clean.run_id = id;\n', ''],
  ['report log from the stored text', 'src/subruns.js', '  clean.log = logInfo;\n', ''],
  ['report schema unchecked', 'src/subruns.js', 'doc.schema !== 1', 'false'],
  [
    'report text unbounded',
    'src/subruns.js',
    'const safe = (v, max = 200) => {\n  const t = text(v, max);\n  return t === null || TOKENISH.test(t) || /specviewperf/i.test(t) ? null : t;\n};',
    "const safe = (v) => (typeof v === 'string' ? v : null);",
  ],
  [
    'test rows unbounded',
    'src/subruns.js',
    'suite: safe(r.suite, 48),\n  name: safe(r.name, 120),\n  result: RESULTS.has(r.result)',
    'suite: r.suite,\n  name: r.name,\n  result: RESULTS.has(r.result)',
  ],
  [
    'subrun reason unbounded',
    'src/subruns.js',
    '    reason: safe(row.reason),\n    stages,',
    '    reason: row.reason,\n    stages,',
  ],
  [
    'old database takes the run page down',
    'src/api.js',
    '      } catch {\n        return empty;\n      }',
    '      } catch (error) {\n        throw error;\n      }',
  ],
  [
    'log url without the origin check',
    'src/api.js',
    "        const home = logsBase(env.LOGS_BASE);\n        if (!home) throw new ApiError(404, 'not_found', 'logs not configured');\n        return redirect(home + '/' + id + '/' + name);",
    "        const home = String(env.LOGS_BASE ?? '');\n        return redirect(home + '/' + id + '/' + name);",
  ],
  ['log redirect without a published log', 'src/api.js', 'if (present) {', 'if (object || true) {'],
  [
    'log identity url unchecked',
    'src/api.js',
    "            ...(base ? { url: base + '/' + id + '/log.txt' } : {}),",
    "            url: (base ?? '') + '/' + id + '/log.txt',",
  ],
  [
    'unknown parameter allowed list',
    'src/validate.js',
    "      throw new ApiError(400, 'bad_request', 'unknown parameter', {\n        allowed: Object.keys(allowed),\n      });",
    "      throw new ApiError(400, 'bad_request', 'unknown parameter');",
  ],
  [
    'cpu pattern',
    'src/validate.js',
    'cpuSpec: /^(?:amd|intel|arm|other)(?:-(?:x86_64|aarch64))?$/,',
    'cpuSpec: /./,',
  ],
  [
    'sort pattern',
    'src/validate.js',
    'subSort: /^(?:closeness|recent|duration)$/,',
    'subSort: /./,',
  ],
  [
    'run page immutable before its report row',
    'src/api.js',
    '      (stored !== null && stored !== undefined\n        ? true\n        : Date.now() - Date.parse(row.ended_at) > PUBLISH_HORIZON_MS);',
    '      true;',
  ],
  [
    'run page never immutable after the horizon',
    'src/api.js',
    'Date.now() - Date.parse(row.ended_at) > PUBLISH_HORIZON_MS);',
    'false);',
  ],
  ['subruns probes unbounded', 'src/subruns.js', '    if (done.size >= MAX_PROBES) break;\n', ''],
  [
    'subruns no shared cache',
    'src/api.js',
    'return json({ api: API_VERSION, ...(await ciSubruns(db, q)) }, { cache: CACHE.edge });',
    'return json({ api: API_VERSION, ...(await ciSubruns(db, q)) });',
  ],
  [
    'stats no shared cache',
    'src/api.js',
    'return json({ api: API_VERSION, ...(await ciStats(db, q)) }, { cache: CACHE.edge });',
    'return json({ api: API_VERSION, ...(await ciStats(db, q)) });',
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
    cpSync(names, join(root, 'site/status/gpu_names.json'));
    mkdirSync(join(root, 'ci/vast-broker'), { recursive: true });
    cpSync(defaults, join(root, 'ci/vast-broker/defaults.json'));
    const target = join(copy, file);
    const original = readFileSync(target, 'utf8');
    if (original.split(before).length !== 2) throw Error('ambiguous or missing mutation: ' + name);
    writeFileSync(target, original.replace(before, after));
    const tests = [
      'api',
      'static',
      'row-tag',
      'fixture',
      'ci',
      'setups',
      'rank',
      'subruns',
      'reports',
    ].map((t) => join(copy, 'test', t + '.test.js'));
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
