import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';

// The result store on workerd's own D1 (Miniflare local SQLite), with the real migrations.
// FALSIFIER: a statement of src/results.js is not valid D1 SQL (INSERT ... SELECT ... ON
// CONFLICT, partial writes), a batch is not atomic, or a final verdict changes on D1.
const bundled = await build({
  entryPoints: [new URL('./store-fixture.js', import.meta.url).pathname],
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});
const MIGRATIONS = new URL('../../platform-api/migrations/', import.meta.url);
const statements = readdirSync(MIGRATIONS)
  .filter((name) => /^[0-9]{4}_[a-z0-9_]+\.sql$/.test(name))
  .sort()
  .flatMap((name) =>
    readFileSync(new URL(name, MIGRATIONS), 'utf8')
      .replace(/--[^\n]*/g, '')
      .split(/;\s*$/m)
      .map((s) => s.trim())
      .filter(Boolean),
  );

async function runtime(t) {
  const mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: bundled.outputFiles[0].text,
      compatibilityDate: '2026-10-21',
      d1Databases: ['DB'],
      bindings: { ROW_TAG_KEY: 'w'.repeat(40) },
    }),
  );
  t.after(() => mf.dispose());
  const db = await mf.getD1Database('DB');
  await db.batch(statements.map((s) => db.prepare(s)));
  const post = async (path, body) => {
    const r = await mf.dispatchFetch('https://store.test' + path, {
      method: 'POST',
      body: JSON.stringify(body),
    });
    return { status: r.status, data: await r.json() };
  };
  return { db, post };
}

const SHA = 'c'.repeat(40);
const TREE = 'a'.repeat(64);
const T0 = Date.UTC(2026, 10, 5, 10);
const row = (patch = {}) => ({
  run_id: 'vast-7-1-0',
  commit_sha: SHA,
  tree_hash: TREE,
  tier: 'middle',
  profile: 'check',
  die: 'AD104',
  platform: 'both',
  verdict: 'PASS',
  stages: [
    { name: 'gates', rc: 0, duration_ms: 5, result: 'PASS' },
    { name: 'windows_gate', rc: 0, duration_ms: 6, result: 'PASS' },
  ],
  gpu_name: 'RTX_4070',
  driver_versions: { host: '580.95.05' },
  machine_id: 4242,
  price_usd_h: 0.3,
  cost_usd: 0.9,
  started_at: T0,
  ended_at: T0 + 3600000,
  ...patch,
});
const REQ =
  'version = 1\n[[require]]\ntier = "middle"\nfamily = "any"\ndies = ["any"]\n' +
  'platforms = ["linux", "windows"]\nstages = ["gates", "windows_gate"]\n';

test('workerd D1: in flight, finalised once, duplicate, conflict, requirement met', async (t) => {
  const { db, post } = await runtime(t);
  const inflight = row({ verdict: null, ended_at: null, stages: [] });
  assert.equal((await post('/record', inflight)).data.state, 'INSERTED');
  const joined = await post('/check', {
    commit_sha: SHA,
    tree_hash: TREE,
    tier: 'middle',
    requirements: REQ,
    now: T0 + 60000,
  });
  assert.deepEqual(joined.data.inflight, ['vast-7-1-0']);
  assert.equal((await post('/record', row())).data.state, 'FINALIZED');
  assert.equal((await post('/record', row())).data.state, 'DUPLICATE');
  assert.equal(
    (await post('/record', row({ verdict: 'FAIL', stages: [] }))).data.state,
    'CONFLICT',
  );
  const stored = await db.prepare('SELECT verdict, tag FROM ci_runs').all();
  assert.equal(stored.results.length, 1);
  assert.equal(stored.results[0].verdict, 'PASS');
  assert.match(stored.results[0].tag, /^[A-Za-z0-9_-]{43}$/);
  const stages = await db.prepare('SELECT name FROM ci_run_stages ORDER BY ord').all();
  assert.deepEqual(
    stages.results.map((s) => s.name),
    ['gates', 'windows_gate'],
  );
  const priv = await db.prepare('SELECT machine_id, cost_usd FROM ci_runs_private').all();
  assert.deepEqual(priv.results, [{ machine_id: 4242, cost_usd: 0.9 }]);
  const met = await post('/check', {
    commit_sha: SHA,
    tree_hash: TREE,
    tier: 'middle',
    requirements: REQ,
    now: T0 + 7200000,
  });
  assert.deepEqual(met.data, { met: true, missing: [], inflight: [] });
});

test('workerd D1: a failing statement rolls the whole batch back', async (t) => {
  const { db, post } = await runtime(t);
  // A trigger makes every stage insert fail inside D1; the finalising UPDATE in the same batch
  // must roll back with it.
  await post('/record', row({ verdict: null, ended_at: null, stages: [] }));
  await db.exec(
    "CREATE TRIGGER boom BEFORE INSERT ON ci_run_stages BEGIN SELECT RAISE(ABORT, 'boom'); END",
  );
  const r = await post('/record', row());
  assert.equal(r.status, 400);
  const stored = await db.prepare('SELECT verdict, tag FROM ci_runs').all();
  assert.deepEqual(stored.results, [{ verdict: null, tag: null }], 'finalisation rolled back');
});
