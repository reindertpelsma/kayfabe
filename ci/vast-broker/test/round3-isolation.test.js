import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { setup, sha } from './helpers.js';
const workflow = new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url);
// FALSIFIER: PR code controls the runner, a fork reaches rent, non-base OIDC
// authorizes the call, or the Worker-talking job has any token besides OIDC.
test('R3 workflow executes only base code via pull_request_target', async () => {
  const text = await readFile(workflow, 'utf8');
  assert.match(text, /  pull_request_target:/);
  assert.doesNotMatch(text, /  pull_request:/);
  assert.match(text, /python3 -I/);
  assert.match(text, /raw\.githubusercontent\.com/);
  assert.match(text, /ci\/vast-broker\/\$file/);
  assert.match(text, /for file in runner.py deployment.json/);
  assert.doesNotMatch(text, /actions\/checkout|upload-artifact|download-artifact|cache@/);
});
test('R3 Worker job is OIDC-only with empty inherited environment', async () => {
  const text = await readFile(workflow, 'utf8');
  const job = text.split('  gpu:\n')[1].split('  report:\n')[0];
  assert.match(job, /permissions:\n      id-token: write\n/);
  assert.doesNotMatch(job, /contents:|pull-requests:|GH_TOKEN|github.token/);
  assert.match(job, /env -i/);
  assert.match(text, /bash --noprofile --norc -eo pipefail/);
  assert.match(job, /timeout-minutes:/);
  assert.match(text, /concurrency:/);
});
for (const fork of [false, true])
  test('R3 target-event admission crosschecks run and exact PR ' + fork, async () => {
    const h = await setup();
    h.cloud.run.event = 'pull_request_target';
    h.cloud.run.head_sha = 'd'.repeat(40);
    h.cloud.run.head_branch = 'master';
    h.cloud.run.pull_requests[0].head.sha = sha;
    if (fork) h.cloud.prs[0].head.repo.full_name = 'fork-owner/kayfabe';
    const response = await h.call('/rent', {
      claims: {
        event_name: 'pull_request_target',
        sha: 'd'.repeat(40),
        ref: 'refs/heads/master',
        workflow_ref: 'reindertpelsma/kayfabe/.github/workflows/gpu-vast.yml@refs/heads/master',
        workflow_sha: 'd'.repeat(40),
      },
      body: { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 },
    });
    assert.equal(response.status, fork ? 403 : 200);
    assert.equal(h.cloud.creates, fork ? 0 : 1);
  });
for (const ref of ['refs/pull/7/merge', 'refs/heads/evil'])
  test('R3 non-base workflow cannot mint accepted admission ' + ref, async () => {
    const h = await setup();
    const r = await h.call('/rent', {
      claims: { workflow_ref: 'reindertpelsma/kayfabe/.github/workflows/gpu-vast.yml@' + ref },
    });
    assert.equal(r.status, 401);
    assert.equal(h.cloud.creates, 0);
  });
for (const mismatch of ['workflow-sha', 'run-sha', 'member-sha', 'member-pr'])
  test('R3 target-event refuses mismatched ' + mismatch, async () => {
    const h = await setup();
    h.cloud.run.event = 'pull_request_target';
    h.cloud.run.head_sha = 'd'.repeat(40);
    h.cloud.run.head_branch = 'master';
    h.cloud.run.pull_requests[0].head.sha = sha;
    const claims = {
      event_name: 'pull_request_target',
      sha: 'd'.repeat(40),
      ref: 'refs/heads/master',
      workflow_ref: 'reindertpelsma/kayfabe/.github/workflows/gpu-vast.yml@refs/heads/master',
      workflow_sha: 'd'.repeat(40),
    };
    if (mismatch === 'workflow-sha') claims.workflow_sha = 'e'.repeat(40);
    if (mismatch === 'run-sha') claims.sha = 'e'.repeat(40);
    if (mismatch === 'member-sha') h.cloud.run.pull_requests[0].head.sha = 'e'.repeat(40);
    if (mismatch === 'member-pr') h.cloud.run.pull_requests[0].number = 8;
    const r = await h.call('/rent', {
      claims,
      body: { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 },
    });
    assert.equal(r.status, 401);
    assert.equal(h.cloud.creates, 0);
  });
for (const authority of ['claims', 'GitHub-run'])
  test('R3 consistent signed BASE pair still crosschecks ' + authority, async () => {
    const h = await setup();
    const signed = authority === 'claims' ? 'e'.repeat(40) : 'd'.repeat(40);
    h.cloud.run.event = 'pull_request_target';
    h.cloud.run.head_sha = authority === 'GitHub-run' ? 'e'.repeat(40) : 'd'.repeat(40);
    h.cloud.run.head_branch = 'master';
    h.cloud.run.pull_requests[0].head.sha = sha;
    const response = await h.call('/rent', {
      claims: {
        event_name: 'pull_request_target',
        sha: signed,
        workflow_sha: signed,
        ref: 'refs/heads/master',
      },
      body: { runner_nonce: 'f'.repeat(64), tested_sha: sha, pr_number: 7 },
    });
    assert.equal(response.status, 401);
    assert.equal(h.cloud.creates, 0);
    assert.ok(h.cloud.calls.some((c) => c.url.includes('/actions/runs/123')));
  });
