import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha, repo, jwt } from './helpers.js';
import { createGateway } from '../src/index.js';
function push(h, branch) {
  Object.assign(h.cloud.run, {
    event: 'push',
    head_branch: branch,
    head_repository: { full_name: repo },
    pull_requests: [],
  });
  h.cloud.prs = [];
  return {
    event_name: 'push',
    ref: 'refs/heads/' + branch,
    sha,
    workflow_sha: sha,
    workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/' + branch,
  };
}
// FALSIFIER H11: valid owner pushes refuse, or any API/signed identity mismatch rents.
for (const branch of ['master', 'feature/owner']) {
  test('round owner push rents exact API-bound head ' + branch, async () => {
    const h = await setup(),
      claims = push(h, branch);
    const response = await h.call('/rent', { claims });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).sha, sha);
    assert.equal(h.cloud.creates, 1);
  });
  for (const defect of ['actor', 'triggering', 'fork', 'sha', 'event', 'attempt', 'completed'])
    test('round push API authority refuses ' + defect + ' ' + branch, async () => {
      const h = await setup(),
        claims = push(h, branch);
      if (defect === 'actor') {
        claims.actor_id = '43';
        h.cloud.run.actor.id = 43;
      }
      if (defect === 'triggering') h.cloud.run.triggering_actor.id = 43;
      if (defect === 'fork') h.cloud.run.head_repository.full_name = 'fork/project';
      if (defect === 'sha') claims.sha = 'e'.repeat(40);
      if (defect === 'event') h.cloud.run.event = 'workflow_run';
      if (defect === 'attempt') h.cloud.run.run_attempt = 2;
      if (defect === 'completed') h.cloud.run.status = 'completed';
      assert.notEqual((await h.call('/rent', { claims })).status, 200);
      assert.equal(h.cloud.creates, 0);
    });
}
for (const route of ['/rent', '/status/123-1'])
  test('round owner branch workflow passes front API proof ' + route, async () => {
    const h = await setup(),
      claims = push(h, 'feature/owner');
    let entries = 0;
    const gateway = createGateway({ fetcher: h.cloud.fetch, now: () => h.time.value });
    const bindings = {
      ...h.bindings,
      LEDGER: {
        idFromName: () => 1,
        get: () => ({
          fetch: async () => {
            entries++;
            return Response.json({});
          },
        }),
      },
    };
    const response = await gateway.fetch(
      new Request(h.bindings.WORKER_URL + '/' + h.bindings.BROKER_PATH_PREFIX + route, {
        method: route === '/rent' ? 'POST' : 'GET',
        headers: { Authorization: 'Bearer ' + (await jwt(claims)) },
      }),
      bindings,
    );
    assert.equal(response.status, 200);
    assert.equal(entries, 1);
  });
for (const relation of ['head', 'ancestor'])
  test('round owner dispatch selects reachable branch commit ' + relation, async () => {
    const h = await setup(),
      selected = 'e'.repeat(40),
      head = 'f'.repeat(40);
    Object.assign(h.cloud.run, {
      event: 'workflow_dispatch',
      head_branch: 'master',
      head_repository: { full_name: repo },
      pull_requests: [],
    });
    h.cloud.prs = [];
    const base = h.cloud.fetch;
    h.cloud.fetch = async (url, init) => {
      if (String(url).endsWith('/commits/' + selected)) return Response.json({ sha: selected });
      if (String(url).includes('/branches?'))
        return Response.json([
          { name: 'feature/selected', commit: { sha: relation === 'head' ? selected : head } },
        ]);
      if (String(url).includes('/compare/'))
        return Response.json({
          status: 'ahead',
          ahead_by: 1,
          behind_by: 0,
          base_commit: { sha: selected },
          merge_base_commit: { sha: selected },
        });
      return base(url, init);
    };
    h.restart();
    const response = await h.call('/rent', {
      claims: { event_name: 'workflow_dispatch', ref: 'refs/heads/master' },
      body: { runner_nonce: 'f'.repeat(64), tested_sha: selected, pr_number: null },
    });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).sha, selected);
  });
for (const branch of ['master', 'owner-workflow'])
  test('round selected dispatch lifecycle keeps workflow context ' + branch, async () => {
    const h = await setup(),
      selected = 'e'.repeat(40);
    Object.assign(h.cloud.run, {
      event: 'workflow_dispatch',
      head_branch: branch,
      head_repository: { full_name: repo },
      pull_requests: [],
    });
    h.cloud.prs = [];
    const base = h.cloud.fetch;
    h.cloud.fetch = async (url, init) => {
      if (String(url).endsWith('/commits/' + selected)) return Response.json({ sha: selected });
      if (String(url).includes('/branches?'))
        return Response.json([{ name: 'selected', commit: { sha: selected } }]);
      return base(url, init);
    };
    h.restart();
    const claims = {
      event_name: 'workflow_dispatch',
      ref: 'refs/heads/' + branch,
      workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/' + branch,
    };
    assert.equal(
      (
        await h.call('/rent', {
          claims,
          body: { runner_nonce: 'f'.repeat(64), tested_sha: selected, pr_number: null },
        })
      ).status,
      200,
    );
    const status = await h.call('/status/123-1', { claims });
    assert.equal(status.status, 200);
    assert.equal((await status.json()).state, 'BOOTING');
    assert.equal(h.cloud.deletes.length, 0);
  });
for (const mode of ['owner_push_or_review', 'review_only'])
  test('round manual selection has no PR caller context ' + mode, async () => {
    const h = await setup({ APPROVAL_MODE: mode }),
      selected = 'e'.repeat(40);
    Object.assign(h.cloud.run, {
      event: 'workflow_dispatch',
      head_branch: 'master',
      head_repository: { full_name: repo },
      pull_requests: [],
    });
    h.cloud.prs[0].head.sha = selected;
    h.cloud.reviews = [{ id: 1, user: { id: 42 }, state: 'APPROVED', commit_id: selected }];
    const base = h.cloud.fetch;
    h.cloud.fetch = async (url, init) => {
      if (String(url).endsWith('/commits/' + selected)) return Response.json({ sha: selected });
      if (String(url).includes('/branches?'))
        return Response.json([{ name: 'owner-pr', commit: { sha: selected } }]);
      return base(url, init);
    };
    h.restart();
    const r = await h.call('/rent', {
      claims: { event_name: 'workflow_dispatch', ref: 'refs/heads/master' },
      body: { runner_nonce: 'f'.repeat(64), tested_sha: selected, pr_number: null },
    });
    assert.equal(r.status, 200);
    assert.equal((await r.json()).pr, null);
  });
for (const reason of ['unreachable', 'diverged', 'merge-base', 'bad-branch'])
  test('round dispatch refuses unproven reachability ' + reason, async () => {
    const h = await setup(),
      selected = 'e'.repeat(40),
      head = 'f'.repeat(40);
    Object.assign(h.cloud.run, {
      event: 'workflow_dispatch',
      head_branch: 'master',
      head_repository: { full_name: repo },
      pull_requests: [],
    });
    h.cloud.prs = [];
    const base = h.cloud.fetch;
    h.cloud.fetch = async (url, init) => {
      if (String(url).endsWith('/commits/' + selected)) return Response.json({ sha: selected });
      if (String(url).includes('/branches?'))
        return Response.json(
          reason === 'unreachable'
            ? []
            : [{ name: 'selected', commit: { sha: reason === 'bad-branch' ? 'bad' : head } }],
        );
      if (String(url).includes('/compare/'))
        return Response.json({
          status: reason === 'diverged' ? 'diverged' : 'ahead',
          ahead_by: 1,
          behind_by: 0,
          base_commit: { sha: selected },
          merge_base_commit: { sha: 'd'.repeat(40) },
        });
      return base(url, init);
    };
    h.restart();
    const r = await h.call('/rent', {
      claims: { event_name: 'workflow_dispatch', ref: 'refs/heads/master' },
      body: { runner_nonce: 'f'.repeat(64), tested_sha: selected, pr_number: null },
    });
    assert.notEqual(r.status, 200);
    assert.equal(h.cloud.creates, 0);
  });
for (const repeated of [false, true])
  test('round reachability API work is bounded ' + repeated, async () => {
    const { Github } = await import('../src/github.js');
    let comparisons = 0,
      lists = 0;
    const client = new Github(
      async (url) => {
        if (url.includes('/branches?')) {
          const page = lists++;
          return Response.json(
            Array.from({ length: 100 }, (_, i) => ({
              name: 'branch-' + i,
              commit: { sha: ((repeated ? 0 : page * 100) + i + 1).toString(16).padStart(40, '0') },
            })),
          );
        }
        comparisons++;
        return Response.json({ status: 'diverged' });
      },
      { repo },
    );
    assert.equal(await client.reachable('e'.repeat(40)), false);
    assert.ok(lists <= 5);
    assert.ok(comparisons <= 16);
  });
