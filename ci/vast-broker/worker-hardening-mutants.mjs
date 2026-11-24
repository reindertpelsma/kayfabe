import { cpSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const broker = fileURLToPath(new URL('.', import.meta.url)),
  repo = fileURLToPath(new URL('../..', import.meta.url));
// The mutant copy is a throwaway tree: the test files read sibling trees by relative path
// (ci/vast-reaper, ci/platform-api, scripts/, .github/workflows, site/book-theme), so those are
// copied whole, and node_modules is copied as a real directory (never a symlink: a symlinked one
// changes the bundle hash, scripts/check-bundle-pin.mjs).
const TREES = ['ci', 'scripts', '.github', 'site/book-theme'];
const keep = (p) => !p.includes('/node_modules') && !p.includes('/.wrangler');
const scratch = mkdtempSync(join(tmpdir(), 'kf-hardening-'));
const edit = (name, file, before, after, tests = NODE) => ({ name, file, before, after, tests });
// Review of 2026-11-13 (F4, F5, F6, F8, F10): each mutant undoes one fix and must fail at least two
// tests. Node tests run from test/, the Durable Object ones in a real workerd from workerd/.
const NODE = ['test/worker-hardening.test.js', 'test/round-deploy-helper.test.js'];
const WORKERD = ['workerd/hostbox.test.js', '--test-name-pattern=^(F5|F10)'];
const V = 'src/vast.js',
  R = '../vast-reaper/worker.js',
  RT = 'src/routes.js',
  PL = 'src/publiclog.js',
  I = 'src/index.js',
  H = 'src/hostbox.js',
  D = 'deploy.mjs';
const cases = [
  edit(
    'destroy blocked by a changed label',
    V,
    '(!destroying && record.idSource',
    '(record.idSource',
  ),
  edit('destroy accepts another id', V, 'String(i.id) !== record.id ||', 'false ||'),
  edit(
    'destroy accepts a foreign label prefix',
    V,
    "!record.label.startsWith('kf-ci-') ||",
    'false ||',
  ),
  edit(
    'destroy drops the onstart proof',
    V,
    'if (record.proofVersion === 1 && !record.contractVerified) {',
    'if (false) {',
  ),
  edit(
    'reaper forgets seen ids',
    R,
    '(Number.isSafeInteger(row.id) && seen[row.id] === row.start_date)',
    'false',
  ),
  edit(
    'reaper takes any id',
    R,
    "(typeof row.label === 'string' && LABEL.test(row.label)) ||",
    'true ||',
  ),
  edit(
    'reaper never prunes gone ids',
    R,
    'else if (seen[row.id] === row.start_date) next[row.id] = row.start_date;',
    'else next[row.id] = seen[row.id] ?? row.start_date;',
  ),
  edit(
    'reaper default below the hard limit',
    R,
    "integer('REAPER_HARD_MINUTES', 300, 1, 1440)",
    "integer('REAPER_HARD_MINUTES', 180, 1, 1440)",
  ),
  edit('no nosniff', RT, "headers.set('X-Content-Type-Options', 'nosniff');", ''),
  edit(
    'no CSP',
    RT,
    "headers.set('Content-Security-Policy', \"default-src 'none'; sandbox\");",
    '',
  ),
  edit(
    'no attachment disposition',
    RT,
    "headers.set('Content-Disposition', 'attachment; filename=\"response.txt\"');",
    '',
  ),
  edit(
    'gateway not hardened',
    I,
    'return harden(await gatewayFetch(request, env));',
    'return gatewayFetch(request, env);',
  ),
  edit(
    'public object without disposition',
    PL,
    "  contentDisposition:\n    'inline; filename=\"' + (key.endsWith('.json') ? 'report.json' : 'log.txt') + '\"',",
    '',
  ),
  edit(
    'PASS decays the fault history',
    I,
    "if (r.profile === 'sweep' && payload.result.verdict === 'PASS')",
    "if (r.result.verdict === 'PASS') await ledger.markProviderSuccess(r, cfg.provider);\n        if (r.profile === 'sweep' && payload.result.verdict === 'PASS')",
  ),
  edit(
    'limiter keyed per full IPv6 address',
    I,
    "if (typeof ip !== 'string' || !ip.includes(':') || ip.includes('.')) return String(ip);",
    'return String(ip);',
  ),
  edit(
    'full limiter table refuses',
    I,
    'hostUpgrades.delete(hostUpgrades.keys().next().value);',
    "return new Response('Too many requests', { status: 429 });",
  ),
  edit(
    'deploy accepts extra modules',
    D,
    'const extra = readdirSync(out).filter((n) => !EMITTED.has(n));',
    'const extra = [];',
  ),
  edit(
    'deploy self-report from toml',
    D,
    "const upload = [...vars, ['DEPLOYED_BUNDLE_SHA256', digest]];",
    'const upload = [...vars];',
  ),
  edit(
    'flapping state written at once',
    H,
    'export const HB_CHANGE_MIN_MS = 5000;',
    'export const HB_CHANGE_MIN_MS = 0;',
    WORKERD,
  ),
  edit(
    'close does not flush',
    H,
    '  async webSocketClose(ws, code, reason) {\n    await this.flushHeartbeat(ws, this.now(), true);',
    '  async webSocketClose(ws, code, reason) {',
    WORKERD,
  ),
  edit(
    'alarm does not flush',
    H,
    'flushIn = Math.min(flushIn, (await this.flushHeartbeat(w, now)) ?? Infinity);',
    'void w;',
    WORKERD,
  ),
  edit(
    'disabled seeder still served',
    H,
    '!record || !cfg?.enabled || !cfg.seeder?.enabled',
    '!record',
    WORKERD,
  ),
];
function run(tests) {
  const r = spawnSync(
    process.execPath,
    [
      '--test',
      '--test-reporter=tap',
      ...tests.filter((x) => x.startsWith('--')),
      ...tests.filter((x) => !x.startsWith('--')),
    ],
    {
      cwd: join(scratch, 'ci/vast-broker'),
      encoding: 'utf8',
      timeout: 600000,
      maxBuffer: 32 * 1024 * 1024,
      env: { ...process.env, MINIFLARE_ASSERT_BODIES_CONSUMED: 'true' },
    },
  );
  if (r.error) throw r.error;
  const failures = [];
  for (const block of r.stdout.split(/^# Subtest: /m)) {
    const name = block.match(/^not ok \d+ - (.+)$/m)?.[1];
    if (!name) continue;
    failures.push(name);
  }
  if (r.status !== 0 && !failures.length) throw Error(r.stdout + r.stderr);
  return {
    status: r.status,
    tests: Number(r.stdout.match(/^# tests (\d+)$/m)?.[1]),
    // The 'mutant copy' test checks the copy tree, not the Worker: a mutant that breaks it is not a killer.
    failures: [...new Set(failures)].filter((n) => !n.startsWith('mutant copy:')),
  };
}
try {
  for (const path of TREES) {
    mkdirSync(dirname(join(scratch, path)), { recursive: true });
    cpSync(join(repo, path), join(scratch, path), { recursive: true, filter: keep });
  }
  cpSync(join(broker, 'node_modules'), join(scratch, 'ci/vast-broker/node_modules'), {
    recursive: true,
    verbatimSymlinks: true,
  });
  const baselines = { node: run(NODE), workerd: run(WORKERD) };
  for (const [k, b] of Object.entries(baselines))
    if (b.status !== 0) throw Error('baseline failed: ' + k + JSON.stringify(b));
  const rows = [],
    weak = [];
  for (const c of cases) {
    const path = join(scratch, 'ci/vast-broker', c.file),
      original = readFileSync(path, 'utf8');
    try {
      if (original.split(c.before).length !== 2) throw Error('ambiguous edit ' + c.name);
      writeFileSync(
        path,
        original.replace(c.before, () => c.after),
      );
      const r = run(c.tests);
      rows.push({ mutant: c.name, kills: r.failures.length, killers: r.failures });
      if (r.status !== 1 || r.failures.length < 2) weak.push(c.name);
    } finally {
      writeFileSync(path, original);
    }
  }
  const status = weak.length ? 'FAIL' : 'PASS';
  process.stdout.write(
    JSON.stringify(
      {
        status,
        fewer_than_two_killers: weak,
        baseline_tests: { node: baselines.node.tests, workerd: baselines.workerd.tests },
        cases: rows,
      },
      null,
      2,
    ) + '\n',
  );
  if (weak.length) process.exitCode = 1;
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
