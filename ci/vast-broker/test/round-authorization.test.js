import test from 'node:test';
import assert from 'node:assert/strict';
import { env, repo, sha } from './helpers.js';
import { configuration } from '../src/policy.js';
import { allowedWorkflowRef } from '../src/workflow-authority.js';
const cfg = configuration(env);
const check = repo + '/.github/workflows/gpu-vast.yml@refs/heads/master';
const sweep = repo + '/.github/workflows/gpu-vast-sweep.yml@refs/heads/master';
const claims = (workflow = check, event_name = 'push') => ({
  repository: repo,
  actor_id: '42',
  run_id: '123',
  run_attempt: '1',
  event_name,
  sha,
  ref: 'refs/heads/master',
  workflow_ref: workflow,
  job_workflow_ref: workflow,
});
// FALSIFIER H11 (owner 19:10/19:25): any workflow other than the two pinned master files, or
// an event the file's YAML cannot start, passes the pure pin.
for (const event of ['push', 'pull_request_target'])
  test('round pure workflow authority keeps master check pin ' + event, () => {
    assert.equal(allowedWorkflowRef(claims(check, event), cfg, 'check'), true);
    assert.equal(allowedWorkflowRef(claims(check, event), cfg, 'any'), true);
    assert.equal(allowedWorkflowRef(claims(check, event), cfg, 'sweep'), false);
  });
for (const actor of ['42', '43'])
  test('round pure workflow authority keeps master sweep pin ' + actor, () => {
    const c = { ...claims(sweep, 'workflow_dispatch'), actor_id: actor };
    assert.equal(allowedWorkflowRef(c, cfg, 'sweep'), true);
    assert.equal(allowedWorkflowRef(c, cfg, 'check'), false);
  });
for (const defect of [
  'repository',
  'repository-sweep',
  'path',
  'branch',
  'tag',
  'PR',
  'suffix',
  'case',
  'job-ref',
  'job-ref-evil',
  'check-dispatch',
  'check-workflow_run',
  'sweep-push',
  'profile',
])
  test('round pure workflow authority rejects ' + defect, () => {
    let c = claims(),
      profile = 'any';
    const ref = (value) => (c.workflow_ref = c.job_workflow_ref = value);
    if (defect === 'repository') c.repository = 'other/project';
    if (defect === 'repository-sweep') {
      c = { ...claims(sweep, 'workflow_dispatch'), repository: 'other/project' };
      profile = 'sweep';
    }
    if (defect === 'path') ref(repo + '/.github/workflows/evil.yml@refs/heads/master');
    if (defect === 'branch') ref(repo + '/.github/workflows/gpu-vast.yml@refs/heads/feature');
    if (defect === 'tag') ref(repo + '/.github/workflows/gpu-vast.yml@refs/tags/master');
    if (defect === 'PR') ref(repo + '/.github/workflows/gpu-vast.yml@refs/pull/7/merge');
    if (defect === 'suffix') ref(check + '.x');
    if (defect === 'case') ref(check.replace('gpu-vast', 'GPU-vast'));
    if (defect === 'job-ref') c.job_workflow_ref = sweep;
    if (defect === 'job-ref-evil')
      c.job_workflow_ref = repo + '/.github/workflows/evil.yml@refs/heads/master';
    if (defect === 'check-dispatch') c = claims(check, 'workflow_dispatch');
    if (defect === 'check-workflow_run') c = claims(check, 'workflow_run');
    if (defect === 'sweep-push') c = claims(sweep, 'push');
    if (defect === 'profile') profile = 'other';
    assert.equal(allowedWorkflowRef(c, cfg, profile), false);
  });
