import test from 'node:test';
import assert from 'node:assert/strict';
import { setup } from './helpers.js';
import { tierForClaims } from '../src/tier.js';
import { FakeD1, d1Skip } from './d1.js';
// Review of PR #269, R4 (2026-11-05): admission passes the verified OIDC ref and base branch to the
// tier function. FALSIFIERS: a PR into master is not middle; a PR into another branch is middle; a
// push is not small; a release tag is not large; the sweep is not adhoc; a request body or an
// unverified field changes the tier; the store records a PR rental under another tier.

test('R4: tierForClaims reads the event, ref and base branch of the token', () => {
  const pr = { event_name: 'pull_request_target', ref: 'refs/heads/master' };
  assert.equal(tierForClaims({ ...pr, base_ref: 'master' }, 'check'), 'middle');
  assert.equal(tierForClaims(pr, 'check'), 'middle', 'base from ref');
  assert.equal(tierForClaims({ ...pr, base_ref: '' }, 'check'), 'middle', 'empty base_ref');
  assert.equal(tierForClaims({ ...pr, base_ref: 'next' }, 'check'), 'small');
  assert.equal(
    tierForClaims({ event_name: 'pull_request_target', ref: 'refs/heads/next' }, 'check'),
    'small',
    'base next from ref',
  );
  assert.equal(
    tierForClaims(
      { event_name: 'pull_request', base_ref: 'next', ref: 'refs/pull/9/merge' },
      'check',
    ),
    'small',
  );
  assert.equal(
    tierForClaims({ event_name: 'push', ref: 'refs/heads/master', base_ref: '' }, 'check'),
    'small',
  );
  assert.equal(tierForClaims({ event_name: 'push', ref: 'refs/tags/v1.2.0' }, 'check'), 'large');
  assert.equal(tierForClaims({ event_name: 'push', ref: 'refs/tags/x1' }, 'check'), 'small');
  assert.equal(tierForClaims({ event_name: 'workflow_dispatch' }, 'sweep'), 'adhoc');
  assert.equal(tierForClaims({}, 'check'), 'small');
  assert.equal(tierForClaims({ ...pr, base_ref: 'bad ref with spaces' }, 'check'), 'middle');
});

const TAG_KEY = 't'.repeat(48);
for (const [name, claims, run, tier] of [
  ['PR into master', { base_ref: 'master' }, {}, 'middle'],
  ['PR token without base_ref', {}, {}, 'middle'],
  // Not reachable today (gpu-vast.yml triggers on PRs into master only); the token decides.
  ['PR into next', { base_ref: 'next' }, {}, 'small'],
  [
    'push to master',
    { event_name: 'push', base_ref: '' },
    { event: 'push', head_branch: 'master', pull_requests: [] },
    'small',
  ],
])
  test(
    'R4: a ' + name + ' rental is recorded and stored as ' + tier,
    { skip: d1Skip },
    async () => {
      const db = new FakeD1();
      const h = await setup({ DB: db, ROW_TAG_KEY: TAG_KEY });
      h.cloud.run = { ...h.cloud.run, ...run };
      const r = await h.call('/rent', { claims });
      assert.equal(r.status, 200, await r.clone().text());
      const record = await h.storage.get('record:123-1');
      assert.equal(record.tier, tier);
      await h.storage.put('treekey:' + record.sha, { key: 'f'.repeat(64), kind: 'tree', at: 1 });
      await h.service.sweep();
      assert.deepEqual(db.rows('SELECT run_id, tier FROM ci_runs'), [
        { run_id: 'vast-123-1-0', tier },
      ]);
    },
  );
