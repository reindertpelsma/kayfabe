import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, repo, sha } from './helpers.js';
// FALSIFIER: owner-authorized own branch workflows pass stateless auth but
// a stale master literal in the independent GitHub-facts check rejects them.
for (const base of ['stable', 'integration/owner-approved'])
  test('target event honors owner-authorized branch BASE ' + base, async () => {
    const h = await setup({ ALLOWED_BASE_BRANCHES: JSON.stringify([base]) });
    h.cloud.run.event = 'pull_request_target';
    h.cloud.run.head_sha = 'd'.repeat(40);
    h.cloud.run.head_branch = base;
    h.cloud.prs[0].base.ref = base;
    const response = await h.call('/rent', {
      claims: {
        event_name: 'pull_request_target',
        sha: 'd'.repeat(40),
        ref: 'refs/heads/' + base,
        workflow_sha: 'd'.repeat(40),
        workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/' + base,
      },
      body: { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 },
    });
    assert.equal(response.status, 200);
    assert.equal(h.cloud.creates, 1);
  });
