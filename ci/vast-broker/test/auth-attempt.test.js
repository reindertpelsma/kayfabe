import test from 'node:test';
import assert from 'node:assert/strict';
import { Github } from '../src/github.js';
import { baseClaims, MockCloud, owner, repo, sha } from './helpers.js';

// FALSIFIER: admission already reads the signed attempt and ignores a stale latest-run view.
// The incident's old github:run log cannot establish which field differed historically.
const cfg = { repo, owner, profile: 'check' };
const checks = [
  ['id', { id: 124 }],
  ['run_attempt', { run_attempt: 2 }],
  ['event', { event: 'workflow_dispatch' }],
  ['status', { status: 'completed' }],
  ['repository', { repository: { full_name: 'elsewhere/repo' } }],
  ['head_repository', { head_repository: { full_name: 'elsewhere/repo' } }],
  ['head_sha', { head_sha: 'invalid-sentinel-value' }],
  ['head_branch', { head_branch: null }],
  ['triggering_actor', { triggering_actor: { id: 43 } }],
  ['sha', { head_sha: 'd'.repeat(40) }],
];
function scenario(attempt, exactPatch = {}, latestPatch = {}) {
  const cloud = new MockCloud();
  const exact = { ...cloud.run, event: 'push', run_attempt: attempt, ...exactPatch };
  const latest = { ...cloud.run, event: 'push', run_attempt: attempt, ...latestPatch };
  const paths = [];
  const api = new Github(async (url) => {
    const path = new URL(url).pathname;
    paths.push(path);
    if (path === '/repos/' + repo + '/actions/runs/123/attempts/' + attempt)
      return Response.json(exact);
    if (path === '/repos/' + repo + '/actions/runs/123') return Response.json(latest);
    throw Error('unexpected API path');
  }, cfg);
  const claims = { ...baseClaims, event_name: 'push', run_attempt: String(attempt) };
  return { api, claims, paths };
}
for (const attempt of [3, 4]) {
  for (const previousStatus of ['in_progress', 'completed'])
    test('attempt ' + attempt + ' ignores latest prior attempt ' + previousStatus, async () => {
      const { api, claims, paths } = scenario(
        attempt,
        {},
        {
          run_attempt: attempt - 1,
          status: previousStatus,
        },
      );
      assert.deepEqual(await api.authorize(claims, sha), {
        sha,
        branch: 'owner-pr',
        run: '123',
        attempt: String(attempt),
      });
      assert.deepEqual(paths, ['/repos/' + repo + '/actions/runs/123/attempts/' + attempt]);
    });
  for (const [field, patch] of checks)
    test('attempt ' + attempt + ' exact metadata binds ' + field, async () => {
      const { api, claims } = scenario(attempt, patch);
      await assert.rejects(api.authorize(claims, sha), (error) => {
        assert.equal(error.status, 401);
        assert.equal(error.state, 'UNAUTHORIZED');
        assert.equal(error.why, 'github:run:' + field);
        assert.ok(!JSON.stringify(error).includes('sentinel'));
        return true;
      });
    });
  test('attempt ' + attempt + ' requested SHA still binds to attempt head', async () => {
    const { api, claims } = scenario(attempt);
    await assert.rejects(api.authorize(claims, 'd'.repeat(40)), { status: 409 });
  });
  test('attempt ' + attempt + ' admission is not cached between API reads', async () => {
    const cloud = new MockCloud();
    let calls = 0;
    const api = new Github(
      async () =>
        Response.json({
          ...cloud.run,
          run_attempt: attempt,
          status: ++calls === 1 ? 'in_progress' : 'completed',
        }),
      cfg,
    );
    const claims = { ...baseClaims, run_attempt: String(attempt) };
    await api.authorize(claims, sha);
    await assert.rejects(api.authorize(claims, sha), { status: 401 });
    assert.equal(calls, 2);
  });
}
