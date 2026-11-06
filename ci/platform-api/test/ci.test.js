// Public read endpoints over the CI Worker's result store (migration 0002), on the READ-ONLY
// seeded SQLite file (synthetic rows).
// FALSIFIER: a row tag, a private column (machine id, price, cost) or a requirement snapshot
// appears in any response; an unvalidated tree hash, tier or run id reaches SQL; the matrix is
// not the newest row per tier/family/die/platform; a commit page omits runs that share its tree.
import test from 'node:test';
import assert from 'node:assert/strict';
import { api } from './helpers.js';

import { rows, finalRun, running, shared, CI_ENDPOINTS } from './ci-endpoints.js';

const { call } = api({ env: { LOGS_BASE: 'https://logs.kayfabe.dev' } });

test('ci: endpoints answer 200 JSON with CORS * and never leak private data', async () => {
  const privateValues = [
    ...rows.ci_runs_private.map((p) => String(p.machine_id)),
    '0.123',
    '0.456',
    ...rows.ci_runs.filter((r) => r.tag).map((r) => r.tag),
    rows.ci_requirement_snapshots[0].sha256,
    'synthetic requirements snapshot',
  ];
  for (const path of CI_ENDPOINTS()) {
    const r = await call(path);
    assert.equal(r.status, 200, path);
    assert.equal(r.headers.get('access-control-allow-origin'), '*', path);
    assert.doesNotMatch(r.text, /"(?:tag|machine_id|price_usd_h|cost_usd|content)"/, path);
    for (const value of privateValues) assert.ok(!r.text.includes(value), path + ' ' + value);
    assert.doesNotMatch(r.text, /specviewperf|scores/i, path);
  }
  const post = await call('/v1/ci/matrix', { method: 'POST', body: '{}' });
  assert.equal(post.status, 405);
  assert.equal(post.headers.get('access-control-allow-origin'), null);
});

test('ci: parameters are validated before any statement runs', async () => {
  for (const path of [
    '/v1/ci/matrix?tier=huge',
    '/v1/ci/matrix?tree=' + 'A'.repeat(64),
    "/v1/ci/matrix?family=ada'--",
    '/v1/ci/matrix?tree=1&tree=2',
    '/v1/ci/matrix?other=1',
    '/v1/ci/trees?limit=1000',
    '/v1/ci/trees/' + 'g'.repeat(64),
    '/v1/ci/runs/X',
    '/v1/ci/commits/' + 'z'.repeat(40),
  ])
    assert.equal((await call(path)).status, 400, path);
  for (const path of [
    '/v1/ci/trees/' + 'f'.repeat(64),
    '/v1/ci/runs/syn-ci-none-0',
    '/v1/ci/commits/' + 'f'.repeat(40),
    '/v1/ci/nothing',
  ])
    assert.equal((await call(path)).status, 404, path);
});

test('ci: the matrix is the newest row per tier, family, die and platform', async () => {
  const body = (await call('/v1/ci/matrix')).body;
  const expected = new Map();
  for (const r of rows.ci_runs) {
    const key = [r.tier, r.family, r.die, r.platform].join('|');
    const best = expected.get(key);
    if (!best || r.sort_at > best.sort_at || (r.sort_at === best.sort_at && r.run_id > best.run_id))
      expected.set(key, r);
  }
  assert.equal(body.cells.length, expected.size);
  for (const cell of body.cells) {
    const want = expected.get([cell.tier, cell.family, cell.die, cell.platform].join('|'));
    assert.equal(cell.latest.id, want.run_id);
    assert.equal(cell.latest.verdict, want.verdict);
    assert.equal(cell.latest.state, want.verdict === null ? 'running' : 'final');
    if (cell.last_pass) {
      const pass = rows.ci_runs.find((r) => r.run_id === cell.last_pass.id);
      assert.equal(pass.verdict, 'PASS');
    }
  }
  const filtered = (await call('/v1/ci/matrix?tier=middle')).body;
  assert.ok(filtered.cells.every((c) => c.tier === 'middle'));
  assert.ok(filtered.cells.length > 0);
});

test('ci: run page carries stages and host facts; final runs are immutable', async () => {
  const r = await call('/v1/ci/runs/' + finalRun.run_id);
  assert.equal(r.headers.get('cache-control'), 'public, max-age=31536000, immutable');
  const run = r.body.run;
  assert.equal(run.id, finalRun.run_id);
  assert.equal(run.verdict, 'PASS');
  assert.deepEqual(
    run.stages.map((s) => s.name),
    rows.ci_run_stages.filter((s) => s.run_id === finalRun.run_id).map((s) => s.name),
  );
  // The host driver is the box's own driver and each guest has its own: all claims, none of them
  // the provider's offer label (which is not stored or served).
  const claimed = rows.ci_runs.find(
    (r) => r.verdict === 'PASS' && rows.ci_run_claims.some((c) => c.run_id === r.run_id),
  );
  const claims = rows.ci_run_claims.filter((c) => c.run_id === claimed.run_id);
  const host = (await call('/v1/ci/runs/' + claimed.run_id)).body.run.host;
  assert.equal(host.host_driver, claims[0].host_driver);
  assert.deepEqual(
    host.guests.map((g) => [g.os, g.os_version, g.driver]),
    claims.map((c) => [c.guest_os, c.guest_os_version, c.guest_driver]),
  );
  assert.deepEqual(host.claimed, ['cpu_vendor', 'host_driver', 'guest_driver']);
  assert.equal(host.drivers, undefined);
  assert.doesNotMatch(JSON.stringify(host), /"guest":|cpu_model|SECRET/);
  // A run without claims says nothing instead of inventing a driver.
  const bare = rows.ci_runs.find(
    (r) => r.verdict !== null && !rows.ci_run_claims.some((c) => c.run_id === r.run_id),
  );
  const none = (await call('/v1/ci/runs/' + bare.run_id)).body.run.host;
  assert.deepEqual([none.host_driver, none.guests], [null, []]);
  assert.equal(run.log, null);
  const live = await call('/v1/ci/runs/' + running.run_id);
  assert.equal(live.headers.get('cache-control'), 'public, max-age=30');
  assert.equal(live.body.run.state, 'running');
});

test('ci: tree and commit pages show the runs every commit of a tree shares', async () => {
  assert.ok(shared, 'the seed has a tree shared by two commits');
  const [hash, commits] = shared;
  const tree = (await call('/v1/ci/trees/' + hash)).body;
  assert.deepEqual(new Set(tree.commits.map((c) => c.sha)), commits);
  const all = rows.ci_runs.filter((r) => r.tree_hash === hash).map((r) => r.run_id);
  assert.deepEqual(new Set(tree.runs.map((r) => r.id)), new Set(all));
  for (const sha of commits) {
    const page = (await call('/v1/ci/commits/' + sha)).body;
    assert.equal(page.trees[0].hash, hash);
    assert.deepEqual(new Set(page.runs.map((r) => r.id)), new Set(all));
    for (const r of page.runs) assert.equal(r.other_commit, r.commit !== sha);
  }
  const trees = (await call('/v1/ci/trees?limit=3')).body.trees;
  assert.equal(trees.length, 3);
  assert.ok(trees[0].last_at >= trees[1].last_at);
});
