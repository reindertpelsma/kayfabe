import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha, repo } from './helpers.js';
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
  test('round owner push rents exact API-bound head only on master ' + branch, async () => {
    const h = await setup(),
      claims = push(h, branch);
    const response = await h.call('/rent', { claims });
    assert.equal(response.status === 200, branch === 'master');
    if (branch === 'master') assert.equal((await response.json()).sha, sha);
    assert.equal(h.cloud.creates, branch === 'master' ? 1 : 0);
  });
}
// Every run fact is checked for both events the master workflow may start.
for (const event of ['push', 'pull_request_target'])
  for (const defect of ['triggering', 'fork', 'run-repo', 'sha', 'event', 'attempt', 'completed'])
    test('round run API authority refuses ' + defect + ' ' + event, async () => {
      const h = await setup(),
        claims = event === 'push' ? push(h, 'master') : {};
      if (defect === 'triggering') h.cloud.run.triggering_actor.id = 43;
      if (defect === 'fork') h.cloud.run.head_repository = { full_name: 'fork/project' };
      if (defect === 'run-repo') h.cloud.run.repository = { full_name: 'other/project' };
      if (defect === 'sha' && event === 'push') claims.sha = 'e'.repeat(40);
      if (defect === 'sha' && event !== 'push') h.cloud.run.head_sha = 'e'.repeat(40);
      if (defect === 'event') h.cloud.run.event = 'workflow_run';
      if (defect === 'attempt') h.cloud.run.run_attempt = 2;
      if (defect === 'completed') h.cloud.run.status = 'completed';
      const response = await h.call('/rent', {
        claims,
        body: {
          runner_nonce: 'f'.repeat(64),
          tested_sha: sha,
          profile: 'check',
          remaining_seconds: 12000,
        },
      });
      assert.notEqual(response.status, 200);
      assert.equal(h.cloud.creates, 0);
    });
