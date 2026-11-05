import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { TIERS, tier, tierEnvironment, hardLimit, applyProfile } from '../src/tier.js';
import { setup, env } from './helpers.js';
// Lane 2, owner 2026-11-04: Windows is required on the master gate (profile check: gpu-vast.yml on
// a master PR or push), one box, no SPECviewperf; the owner-dispatched die sweep stays Linux-only.
// FALSIFIER (written before the code): the sweep box gets KF_MERGE_WINDOWS=1, the check box gets
// anything but 1, a request field can change it, an unknown profile maps to a tier, or the check
// profile keeps a hard limit too short for the Linux bar plus the Windows install and gate while
// the sweep's grows beyond 180 minutes by default.
const hex = (n) => n.toString(16).padStart(2, '0').repeat(20);
const sweepClaims = (sha) => ({
  event_name: 'workflow_dispatch',
  sha,
  ref: 'refs/heads/master',
  workflow_ref: env.REPOSITORY + '/.github/workflows/gpu-vast-sweep.yml@refs/heads/master',
});
function runAs(h, n, profile) {
  const sweep = profile === 'sweep';
  h.cloud.runs.set(String(n), {
    ...h.cloud.run,
    id: n,
    head_sha: hex(n),
    head_branch: sweep ? 'master' : 'branch-' + n,
    event: sweep ? 'workflow_dispatch' : 'pull_request_target',
    pull_requests: [],
  });
  return { run_id: String(n), ...(sweep ? sweepClaims(hex(n)) : {}) };
}
async function rentAs(h, n, profile, extra = {}) {
  const response = await h.call('/rent', {
    claims: runAs(h, n, profile),
    body: {
      runner_nonce: 'f'.repeat(64),
      tested_sha: hex(n),
      profile,
      remaining_seconds: 12000,
      ...(profile === 'sweep' ? { preferences: ['GA102'] } : {}),
      ...extra,
    },
  });
  return { status: response.status, data: await response.json() };
}
const market = (h) => {
  h.cloud.offer.host_id = 3001;
  h.cloud.extraOffers = [
    {
      ...h.cloud.offer,
      id: 12,
      gpu_name: 'RTX_3080',
      compute_cap: 860,
      machine_id: 200012,
      host_id: 3012,
    },
  ];
  h.cloud.contracts = [901, 902];
};
const createdEnv = (h) =>
  h.cloud.calls.filter((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url)).map((c) => c.body.env);

test('profile table: check is the Windows master gate, sweep is Linux-only, release is Windows', () => {
  // 2026-11-05 (wtier): profile `release` carries the large tier's legs (admin start only).
  assert.deepEqual(Object.keys(TIERS).sort(), ['check', 'release', 'sweep']);
  assert.deepEqual(tierEnvironment('check'), { KF_MERGE_WINDOWS: '1' });
  assert.deepEqual(tierEnvironment('sweep'), { KF_MERGE_WINDOWS: '0' });
  assert.deepEqual(tierEnvironment('release'), { KF_MERGE_WINDOWS: '1' });
  for (const bad of ['large', '', undefined, 'constructor', '__proto__', 'toString'])
    assert.throws(() => tier(bad), /unknown profile/, String(bad));
  assert.ok(Object.isFrozen(TIERS) && Object.values(TIERS).every(Object.isFrozen));
});

test('per-profile hard limits: check 300 min by default, sweep min(180, check) unless set', () => {
  assert.equal(DEFAULTS.hardMinutes, 300);
  assert.equal(DEFAULTS.sweepHardMinutes, 180);
  const bare = configuration({ ...env, HARD_RUNTIME_MINUTES: undefined });
  assert.equal(hardLimit(bare, 'check'), 300 * 60000);
  assert.equal(hardLimit(bare, 'sweep'), 180 * 60000);
  // An owner who lowers the check limit lowers the default sweep limit with it.
  const low = configuration({ ...env, HARD_RUNTIME_MINUTES: '20' });
  assert.equal(hardLimit(low, 'sweep'), 20 * 60000);
  const own = configuration({
    ...env,
    HARD_RUNTIME_MINUTES: undefined,
    SWEEP_HARD_RUNTIME_MINUTES: '200',
  });
  assert.equal(hardLimit(own, 'sweep'), 200 * 60000);
  for (const bad of ['0', '5', '1441', 'x', '1.5'])
    assert.throws(() => configuration({ ...env, SWEEP_HARD_RUNTIME_MINUTES: bad }), String(bad));
  const applied = applyProfile(configuration({ ...env, HARD_RUNTIME_MINUTES: undefined }), 'sweep');
  assert.deepEqual(
    [applied.profile, applied.hard, applied.ttl],
    ['sweep', 180 * 60000, 180 * 60000],
  );
});

test('the Worker writes the tier of the profile into the box environment, and only it', async () => {
  const h = await setup({
    MAX_CONCURRENT_RENTALS: '2',
    HARD_RUNTIME_MINUTES: '180',
    SWEEP_HARD_RUNTIME_MINUTES: '20',
  });
  market(h);
  // The sweep first: its preference (GA102) is also the middle rotation's Ampere choice.
  assert.equal((await rentAs(h, 202, 'sweep')).status, 200);
  assert.equal((await rentAs(h, 201, 'check')).status, 200);
  const [sweep, check] = createdEnv(h);
  assert.equal(check.KF_MERGE_WINDOWS, '1');
  assert.equal(sweep.KF_MERGE_WINDOWS, '0');
  // The tier comes from the verified event: a master PR is middle, the dispatched sweep adhoc.
  assert.equal(check.KF_TIER, 'middle');
  assert.equal(sweep.KF_TIER, 'adhoc');
  for (const e of [check, sweep]) assert.equal(e.KF_WINDOWS_RUNS ?? e.KF_STAGES, undefined);
  // Each profile's own hard limit sets the box deadlines.
  assert.equal(check.KF_HARD_DEADLINE - check.KF_BOOT_DEADLINE, (180 - 10) * 60);
  assert.equal(sweep.KF_HARD_DEADLINE - sweep.KF_BOOT_DEADLINE, (20 - 10) * 60);
  const c = await h.storage.get('record:201-1'),
    s = await h.storage.get('record:202-1');
  assert.equal(c.hardDeadline - c.started, 180 * 60000);
  assert.equal(s.hardDeadline - s.started, 20 * 60000);
});

test('a request cannot choose the tier', async () => {
  const h = await setup({ HARD_RUNTIME_MINUTES: '180' });
  market(h);
  for (const [n, extra] of [
    [31, { tier: 'sweep' }],
    [32, { KF_MERGE_WINDOWS: '0' }],
  ]) {
    const reply = await rentAs(h, n, 'check', extra);
    assert.deepEqual([reply.status, reply.data.state], [400, 'BAD_REQUEST'], JSON.stringify(extra));
  }
  assert.equal(h.cloud.creates, 0);
});

// The independent reaper deletes kf-ci boxes older than its own hard limit plus margin; below the
// main Worker's check limit it would delete a live Windows master gate (DEPLOY.md: raise it first).
test('the reaper hard limit covers the longest profile hard limit', async () => {
  const { readFile } = await import('node:fs/promises');
  const toml = await readFile(new URL('../../vast-reaper/wrangler.toml', import.meta.url), 'utf8');
  const reaper = Number(/^REAPER_HARD_MINUTES = "(\d+)"$/m.exec(toml)[1]);
  assert.ok(reaper >= DEFAULTS.hardMinutes && reaper >= DEFAULTS.sweepHardMinutes, String(reaper));
});

test('the check profile refuses a runner whose wait budget cannot hold its hard limit', async () => {
  const h = await setup({ HARD_RUNTIME_MINUTES: '300' });
  market(h);
  // 12000 s (200 min) < 300 + 5 min: the master runner of before this change cannot rent.
  const reply = await rentAs(h, 41, 'check');
  assert.deepEqual(
    [reply.status, reply.data.state],
    [409, 'NOT_ELIGIBLE'],
    JSON.stringify(reply.data),
  );
  assert.equal(h.cloud.creates, 0);
  const ok = await rentAs(h, 42, 'check', { remaining_seconds: 330 * 60 });
  assert.equal(ok.status, 200);
});
