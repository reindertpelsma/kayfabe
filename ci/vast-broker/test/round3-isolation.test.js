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
// MEASURED shape (public pull_request_target runs): run.head_sha is the PR head commit and
// head_repository is the PR's head repository; the OIDC sha claim names the BASE commit.
const target = (h, change) => {
  Object.assign(h.cloud.run, { event: 'pull_request_target', head_sha: sha });
  if (change === 'fork') h.cloud.run.head_repository = { full_name: 'fork-owner/kayfabe' };
  if (change === 'run-head') h.cloud.run.head_sha = 'e'.repeat(40);
  return h.call('/rent', {
    claims: {
      event_name: 'pull_request_target',
      sha: 'd'.repeat(40),
      ref: 'refs/heads/master',
      workflow_ref: 'reindertpelsma/kayfabe/.github/workflows/gpu-vast.yml@refs/heads/master',
      workflow_sha: 'd'.repeat(40),
    },
    body: {
      runner_nonce: 'f'.repeat(64),
      tested_sha: change === 'tested-sha' ? 'e'.repeat(40) : sha,
      profile: 'check',
    },
  });
};
for (const fork of [false, true])
  test('R3 target-event admission crosschecks run head and repository ' + fork, async () => {
    const h = await setup();
    const response = await target(h, fork ? 'fork' : null);
    assert.equal(response.status, fork ? 401 : 200);
    assert.equal(h.cloud.creates, fork ? 0 : 1);
  });
for (const mismatch of ['run-head', 'tested-sha'])
  test('R3 target-event refuses mismatched ' + mismatch, async () => {
    const h = await setup();
    const r = await target(h, mismatch);
    assert.equal(r.status, 409);
    assert.equal(h.cloud.creates, 0);
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
import { createGateway } from '../src/index.js';
import { env, clock, jwt, jwk, repo } from './helpers.js';
// FALSIFIER: matched but unapproved job/workflow refs enter the ledger before API policy.
for (const path of ['/rent', '/status/123-1'])
  test('R3 front pins matched workflow identities ' + path, async () => {
    let entries = 0;
    const gateway = createGateway({
      now: () => clock,
      fetcher: async () => Response.json({ keys: [jwk] }),
    });
    const bindings = {
      ...env,
      BROKER_PATH_PREFIX: 'g'.repeat(32),
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
    const workflow_ref = repo + '/.github/workflows/evil.yml@refs/heads/master';
    const response = await gateway.fetch(
      new Request(env.WORKER_URL + '/' + 'g'.repeat(32) + path, {
        method: path === '/rent' ? 'POST' : 'GET',
        headers: {
          Authorization: 'Bearer ' + (await jwt({ workflow_ref, job_workflow_ref: workflow_ref })),
        },
      }),
      bindings,
    );
    assert.equal(response.status, 401);
    assert.equal(entries, 0);
  });
