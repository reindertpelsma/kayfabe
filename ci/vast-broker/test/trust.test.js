import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, repo } from './helpers.js';
import {
  TRUST,
  trustConfigProblems,
  trustDecision,
  normalizeScopes,
  hasPrivateScope,
} from '../src/trust.js';
// FALSIFIER: a run whose provenance is a fork, an unknown head, an unlisted event, branch or actor, or has none at all, receives the private scope;
// or a trusted run (the owner's repository: next, master, the owner's branches, a pull request head of that repository, an admin start of a
// reachable commit) does not; or a token that carries no, an unknown, a duplicated or a private-only scope list is read as private.
const github = (extra = {}) => ({
  source: 'github',
  repository: repo,
  headRepository: repo,
  event: 'push',
  headBranch: 'next',
  actorIsOwner: true,
  ...extra,
});
const admin = (extra = {}) => ({
  source: 'admin',
  repository: repo,
  headRepository: repo,
  event: 'admin_start',
  reachable: true,
  actorIsOwner: true,
  ...extra,
});
const decide = (p) => trustDecision(p, { repository: repo });
const PRIVATE = ['public', 'private'];

test('trust config: the shipped file is good, the safety switches cannot be turned off, a bare wildcard is refused', () => {
  assert.deepEqual(trustConfigProblems(TRUST), []);
  const edit = (fn) => {
    const c = structuredClone(TRUST);
    fn(c);
    return trustConfigProblems(c);
  };
  assert.ok(edit((c) => (c.private.same_repository = false)).includes('SWITCH:same_repository'));
  assert.ok(
    edit((c) => (c.private.require_owner_actor = false)).includes('SWITCH:require_owner_actor'),
  );
  assert.ok(
    edit((c) => (c.private.admin_requires_reachable = false)).includes(
      'SWITCH:admin_requires_reachable',
    ),
  );
  assert.ok(edit((c) => c.private.branches.push('*')).includes('BRANCH_WILDCARD'));
  assert.ok(edit((c) => (c.private.branches = [])).includes('BRANCHES'));
  assert.ok(edit((c) => (c.private.events.github = [])).includes('EVENTS'));
  assert.ok(edit((c) => (c.private.extra = 1)).includes('PRIVATE_KEYS'));
  assert.ok(edit((c) => (c.token_lifetime = 1)).includes('SCHEMA'));
});
test('trust: the trusted refs are data: next, master and the owner branches', () => {
  assert.deepEqual(TRUST.private.branches, ['next', 'master', 'claude/*', 'codex/*']);
});
for (const [label, provenance, why] of [
  ['push to next', github(), 'TRUSTED:branch'],
  ['push to master', github({ headBranch: 'master' }), 'TRUSTED:branch'],
  [
    'push to an owner agent branch',
    github({ headBranch: 'claude/topic-20261117' }),
    'TRUSTED:branch',
  ],
  ['push to a codex branch', github({ headBranch: 'codex/topic-20261117' }), 'TRUSTED:branch'],
  [
    'a collaborator pull request head of this repository',
    github({ event: 'pull_request_target', headBranch: 'feature-x' }),
    'TRUSTED:collaborator_head',
  ],
  [
    'a pull_request head of this repository',
    github({ event: 'pull_request', headBranch: 'feature-x' }),
    'TRUSTED:collaborator_head',
  ],
  ['a manual dispatch on next', github({ event: 'workflow_dispatch' }), 'TRUSTED:branch'],
  ['the owner admin start', admin(), 'TRUSTED:admin'],
]) {
  test('trust: trusted provenance gets the private scope: ' + label, () => {
    const d = decide(provenance);
    assert.deepEqual(d.scopes, PRIVATE);
    assert.equal(d.trusted, true);
    assert.equal(d.why, why);
  });
}
for (const [label, provenance, why] of [
  [
    'a fork pull request (pull_request_target)',
    github({ event: 'pull_request_target', headRepository: 'someone/kayfabe', headBranch: 'next' }),
    'FORK_HEAD',
  ],
  [
    'a fork pull request (pull_request)',
    github({ event: 'pull_request', headRepository: 'someone/kayfabe' }),
    'FORK_HEAD',
  ],
  [
    'a fork whose head branch is called next',
    github({ headRepository: 'evil/kayfabe', headBranch: 'next' }),
    'FORK_HEAD',
  ],
  [
    'a pull request whose head repository is not stated',
    github({ event: 'pull_request_target', headRepository: undefined }),
    'FORK_HEAD',
  ],
  [
    'a head repository that is null (a deleted fork)',
    github({ event: 'pull_request_target', headRepository: null }),
    'FORK_HEAD',
  ],
  [
    'another repository',
    github({ repository: 'someone/kayfabe', headRepository: 'someone/kayfabe' }),
    'REPOSITORY',
  ],
  ['an unlisted event', github({ event: 'issue_comment' }), 'EVENT'],
  ['a schedule', github({ event: 'schedule' }), 'EVENT'],
  ['an admin event name on the github source', github({ event: 'admin_start' }), 'EVENT'],
  ['an actor that is not the owner', github({ actorIsOwner: false }), 'ACTOR'],
  ['an actor that is not stated', github({ actorIsOwner: undefined }), 'ACTOR'],
  ['a push to an unlisted branch', github({ headBranch: 'feature-x' }), 'BRANCH'],
  [
    'a push to a branch that merely starts like a trusted one',
    github({ headBranch: 'nextish' }),
    'BRANCH',
  ],
  ['a push to a branch with no name', github({ headBranch: undefined }), 'BRANCH'],
  ['a branch with a control character', github({ headBranch: 'claude/x\n' }), 'BRANCH'],
  ['a branch of 300 characters', github({ headBranch: 'claude/' + 'a'.repeat(300) }), 'BRANCH'],
  ['an admin start of a commit that is not reachable', admin({ reachable: false }), 'UNREACHABLE'],
  ['an admin start without the reachability fact', admin({ reachable: undefined }), 'UNREACHABLE'],
  ['an admin start whose actor is not stated', admin({ actorIsOwner: undefined }), 'ACTOR'],
  ['an unknown source', github({ source: 'cron' }), 'SOURCE'],
  ['no provenance', undefined, 'NO_PROVENANCE'],
  ['a provenance that is a string', 'trusted', 'NO_PROVENANCE'],
  ['a provenance that is an array', [], 'NO_PROVENANCE'],
]) {
  test('trust: untrusted provenance gets the public scope only: ' + label, () => {
    const d = decide(provenance);
    assert.deepEqual(d.scopes, ['public']);
    assert.equal(d.trusted, false);
    assert.equal(d.why, 'UNTRUSTED:' + why);
  });
}
test('trust: the repository named by the provenance must be this one (head and repository are compared separately)', () => {
  assert.equal(decide(github({ repository: 'someone/kayfabe' })).why, 'UNTRUSTED:REPOSITORY');
  assert.equal(decide(admin({ repository: 'someone/kayfabe' })).why, 'UNTRUSTED:REPOSITORY');
  assert.equal(decide(github({ repository: undefined })).why, 'UNTRUSTED:REPOSITORY');
});
test('trust: no configured repository is untrusted', () => {
  for (const repository of [undefined, '', 'norepo', 'a/b/c'])
    assert.equal(trustDecision(github(), { repository }).trusted, false);
});
test('trust: a pull request head is not trusted when the data turns collaborator heads off', () => {
  const c = structuredClone(TRUST);
  c.private.collaborator_pr_heads = false;
  const p = github({ event: 'pull_request_target', headBranch: 'feature-x' });
  assert.equal(trustDecision(p, { repository: repo, config: c }).trusted, false);
  assert.equal(
    trustDecision(github({ event: 'pull_request_target', headBranch: 'next' }), {
      repository: repo,
      config: c,
    }).trusted,
    true,
  );
});
test('trust: a decision names a fixed reason, never a value', () => {
  const d = decide(github({ headRepository: 'SECRET-OWNER/secret-repo' }));
  assert.ok(!JSON.stringify(d).includes('SECRET'));
});
test('trust: normalizeScopes is fail-closed', () => {
  assert.deepEqual(normalizeScopes(['public', 'private']), PRIVATE);
  assert.deepEqual(normalizeScopes(['public']), ['public']);
  for (const bad of [
    undefined,
    null,
    'private',
    ['private'],
    [],
    ['public', 'public'],
    ['public', 'x'],
    ['public', 'private', 'x'],
    { 0: 'public' },
    7,
  ])
    assert.deepEqual(normalizeScopes(bad), ['public'], JSON.stringify(bad));
  assert.equal(hasPrivateScope(PRIVATE), true);
  assert.equal(hasPrivateScope(['private']), false);
  assert.equal(hasPrivateScope(undefined), false);
});

// ---------------------------------------------------------------- decided at LEASE time, put in the token
function issued(h) {
  const create = h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url));
  const token = create.body.env.KF_RUN_TOKEN;
  return JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString());
}
test('lease: a trusted run gets scopes [public, private] in the signed token and the record', async () => {
  const h = await setup();
  await h.call();
  const record = await h.storage.get('record:123-1');
  assert.deepEqual(record.scopes, PRIVATE);
  assert.equal(record.scopeWhy, 'TRUSTED:collaborator_head');
  assert.deepEqual(issued(h).scopes, PRIVATE);
  assert.equal(record.provenance, undefined, 'the verified provenance is not stored on the record');
});
test('lease: a run whose provenance the data does not trust gets [public] in the token and the record', async () => {
  const h = await setup();
  h.cloud.run = { ...h.cloud.run, event: 'push', head_branch: 'feature-x' };
  await h.call('/rent', { claims: { event_name: 'push' } });
  const record = await h.storage.get('record:123-1');
  assert.deepEqual(record.scopes, ['public']);
  assert.equal(record.scopeWhy, 'UNTRUSTED:BRANCH');
  assert.deepEqual(issued(h).scopes, ['public']);
});
test('lease: a push to next is trusted by branch', async () => {
  const h = await setup();
  h.cloud.run = { ...h.cloud.run, event: 'push', head_branch: 'next' };
  await h.call('/rent', { claims: { event_name: 'push' } });
  assert.equal((await h.storage.get('record:123-1')).scopeWhy, 'TRUSTED:branch');
});
test('lease: the scope is decided at lease time and a later change of the data does not widen a token', async () => {
  const h = await setup();
  h.cloud.run = { ...h.cloud.run, event: 'push', head_branch: 'feature-x' };
  await h.call('/rent', { claims: { event_name: 'push' } });
  const record = await h.storage.get('record:123-1');
  // the route reads the stored decision and the signed claim: both are public
  assert.deepEqual(record.scopes, ['public']);
  assert.deepEqual(issued(h).scopes, ['public']);
});

// ---------------------------------------------------------------- the token itself
import { RunTokens } from '../src/tokens.js';
const secret = 'mock-run-token-binding-only-for-unit-tests';
const fieldsOfToken = async (extra) => {
  const tokens = new RunTokens(secret, () => 1800000000000);
  const token = await tokens.issue({
    run: '123',
    attempt: '1',
    sha: 'a'.repeat(40),
    nonce: 'b'.repeat(32),
    key: '123-1',
    label: 'kf-ci-123-1',
    reportExpiry: 1800001200,
    ...extra,
  });
  return { token, fields: await tokens.verifyStateless(token), tokens };
};
test('token: a token issued with scopes carries them, and reads them back', async () => {
  assert.deepEqual((await fieldsOfToken({ scopes: ['public'] })).fields.scopes, ['public']);
  assert.deepEqual((await fieldsOfToken({ scopes: ['public', 'private'] })).fields.scopes, PRIVATE);
});
test('token: a token issued with no scopes (before the claim existed) reads as public only', async () => {
  const { token, fields } = await fieldsOfToken({});
  assert.deepEqual(fields.scopes, ['public']);
  const payload = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString());
  assert.equal('scopes' in payload, false);
});
test('token: a scope list that is not in normal form is refused at issue, never signed', async () => {
  for (const bad of [['private'], [], ['public', 'public'], ['public', 'x'], 'private'])
    await assert.rejects(
      fieldsOfToken({ scopes: bad }),
      (e) => e.status !== undefined,
      JSON.stringify(bad),
    );
});
