import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha, env } from './helpers.js';
// FALSIFIER (owner 19:25): a sweep tests any commit other than its own run head.
for (const target of [sha, 'e'.repeat(40)])
  test('V5 owner sweep pins the run head commit ' + target[0], async () => {
    const h = await setup(),
      base = h.cloud.fetch;
    h.cloud.run.event = 'workflow_dispatch';
    h.cloud.run.head_branch = 'master';
    h.cloud.prs = [];
    h.cloud.run.pull_requests = [];
    h.cloud.fetch = async (u, i) =>
      new URL(String(u)).pathname.endsWith('/commits/' + target)
        ? Response.json({ sha: target })
        : base(u, i);
    h.restart();
    const r = await h.call('/rent', {
      claims: {
        event_name: 'workflow_dispatch',
        sha,
        ref: 'refs/heads/master',
        workflow_ref: env.REPOSITORY + '/.github/workflows/gpu-vast-sweep.yml@refs/heads/master',
      },
      body: {
        runner_nonce: 'f'.repeat(64),
        tested_sha: target,
        profile: 'sweep',
        remaining_seconds: 12000,
        preferences: ['AD104'],
      },
    });
    assert.equal(r.status, target === sha ? 200 : 409);
    if (target !== sha) return assert.equal(h.cloud.creates, 0);
    assert.equal((await h.storage.get('record:123-1')).sha, target);
    assert.match(h.cloud.instances.get('900').onstart, new RegExp(target));
  });
