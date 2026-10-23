import test from 'node:test';
import assert from 'node:assert/strict';
import { env, repo, sha } from './helpers.js';
import { configuration } from '../src/policy.js';
const cfg = configuration(env);
const base = (branch) => ({
  repository: repo,
  actor_id: '42',
  run_id: '123',
  run_attempt: '1',
  event_name: 'push',
  sha,
  ref: 'refs/heads/' + branch,
  workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/' + branch,
  job_workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/' + branch,
});
const api = (branch) => ({
  id: 123,
  run_attempt: 1,
  event: 'push',
  actor: { id: 42 },
  triggering_actor: { id: 42 },
  repository: { full_name: repo },
  head_repository: { full_name: repo },
  head_sha: sha,
  head_branch: branch,
  status: 'in_progress',
});
async function allowed(c, r, profile = 'any') {
  const { allowedWorkflowRef } = await import('../src/workflow-authority.js');
  return allowedWorkflowRef(c, cfg, r, profile);
}
// FALSIFIER H11: the exception admits any nonowner/fork/unbound workflow/run identity.
for (const branch of ['feature/owner', 'owner-unicode-é']) {
  test('round pure workflow authority allows owner branch ' + branch, async () => {
    assert.equal(await allowed(base(branch), api(branch)), true);
  });
  for (const defect of [
    'repository',
    'path',
    'tag',
    'PR',
    'actor',
    'triggering',
    'fork',
    'run-repo',
    'sha',
    'run-id',
    'attempt',
    'event',
    'job-ref',
    'branch',
  ])
    test('round pure workflow authority rejects ' + defect + ' ' + branch, async () => {
      const c = base(branch),
        r = api(branch);
      if (defect === 'repository') c.repository = 'other/project';
      if (defect === 'path')
        c.workflow_ref = c.job_workflow_ref = repo + '/.github/workflows/evil.yml@' + c.ref;
      if (defect === 'tag')
        c.workflow_ref = c.job_workflow_ref =
          repo + '/.github/workflows/gpu-vast.yml@refs/tags/release';
      if (defect === 'PR')
        c.workflow_ref = c.job_workflow_ref =
          repo + '/.github/workflows/gpu-vast.yml@refs/pull/7/merge';
      if (defect === 'actor') {
        c.actor_id = '43';
        r.actor.id = 43;
      }
      if (defect === 'triggering') r.triggering_actor.id = 43;
      if (defect === 'fork') r.head_repository.full_name = 'fork/project';
      if (defect === 'run-repo') r.repository.full_name = 'other/project';
      if (defect === 'sha') r.head_sha = 'd'.repeat(40);
      if (defect === 'run-id') r.id = 124;
      if (defect === 'attempt') r.run_attempt = 2;
      if (defect === 'event') r.event = 'workflow_run';
      if (defect === 'job-ref')
        c.job_workflow_ref = repo + '/.github/workflows/gpu-vast.yml@refs/heads/master';
      if (defect === 'branch') r.head_branch = 'other';
      assert.equal(await allowed(c, r), false);
    });
}
for (const actor of ['42', '43'])
  test('round pure workflow authority keeps master pin ' + actor, async () => {
    const c = { ...base('master'), actor_id: actor };
    assert.equal(await allowed(c, undefined, 'check'), true);
    assert.equal(await allowed(c, undefined, 'sweep'), false);
  });
