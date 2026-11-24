// The scope of a run token, decided at LEASE time (docs/design/V3_WORKER_PRIVATE_PRESIGN.md). A run of untrusted provenance
// (a fork, an unknown head, an event or branch the data does not list) gets scopes ['public']; only trusted provenance gets
// ['public', 'private']. Private objects are licence-restricted, not sensitive: untrusted code on a box must not be able to pull them
// (the private store does not trust such a run, and the run does not trust the private store), so the decision is made once from facts
// the Worker verified itself (src/github.js authorize: the signed OIDC claims and the read of the signed run attempt; the owner's
// authenticated admin start) and written into the token. The private route only checks the scope. The rules are data (trust.json).
import config from '../trust.json' with { type: 'json' };

export const TRUST = config;
export const SCOPES = Object.freeze(['public', 'private']);
const BRANCH = /^[A-Za-z0-9._/-]{1,200}$/;
const NAME = /^[A-Za-z0-9._-]+\/[A-Za-z0-9._-]+$/;
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const strings = (v) =>
  Array.isArray(v) && v.length > 0 && v.every((x) => typeof x === 'string' && x);

// Every problem of a trust configuration, as a named token; an empty list is a good configuration.
export function trustConfigProblems(value = TRUST) {
  const problems = [];
  const bad = (reason, ok = false) => {
    if (!ok) problems.push(reason);
    return ok;
  };
  if (!bad('SCHEMA', isObject(value) && value.schema === 'kayfabe.worker-trust/1')) return problems;
  bad(
    'SCHEMA',
    Object.keys(value).every((k) => ['schema', 'status', 'private'].includes(k)),
  );
  const p = value.private;
  if (!bad('PRIVATE', isObject(p))) return problems;
  const keys = [
    'same_repository',
    'events',
    'branches',
    'collaborator_pr_heads',
    'require_owner_actor',
    'admin_requires_reachable',
  ];
  bad('PRIVATE_KEYS', Object.keys(p).sort().join() === [...keys].sort().join());
  // the three safety switches may only be on: a configuration that turns one off is refused, never honoured
  for (const k of ['same_repository', 'require_owner_actor', 'admin_requires_reachable'])
    bad('SWITCH:' + k, p[k] === true);
  bad('COLLABORATOR_PR_HEADS', typeof p.collaborator_pr_heads === 'boolean');
  bad(
    'EVENTS',
    isObject(p.events) &&
      Object.keys(p.events).sort().join() === 'admin,github' &&
      strings(p.events.github) &&
      strings(p.events.admin),
  );
  bad(
    'BRANCHES',
    strings(p.branches) && p.branches.every((b) => /^[A-Za-z0-9._/-]{1,100}\*?$/.test(b)),
  );
  // a bare wildcard would make every branch of the repository trusted
  bad('BRANCH_WILDCARD', !(p.branches ?? []).some((b) => b === '*' || b === '/*'));
  return problems;
}
export function checkTrustConfig(value = TRUST) {
  const problems = trustConfigProblems(value);
  if (problems.length) throw Error('trust config: ' + problems[0]);
  return value;
}
const branchMatches = (pattern, name) =>
  pattern.endsWith('*') ? name.startsWith(pattern.slice(0, -1)) : name === pattern;

// The scopes of a new run token. `provenance` is built by the Worker from verified facts:
//   github: { source: 'github', repository, headRepository, event, headBranch, actorIsOwner }
//   admin:  { source: 'admin', repository, headRepository, event: 'admin_start', reachable, actorIsOwner }
// `repository` (the one this Worker serves) comes from the Worker's configuration. Anything missing, malformed or not listed is
// UNTRUSTED (fail closed): the result is ['public'] with a fixed token naming the first reason, never a value.
export function trustDecision(provenance, { repository, config: value = TRUST } = {}) {
  checkTrustConfig(value);
  const p = value.private;
  const no = (why) => ({ scopes: ['public'], trusted: false, why: 'UNTRUSTED:' + why });
  if (!isObject(provenance)) return no('NO_PROVENANCE');
  if (!['github', 'admin'].includes(provenance.source)) return no('SOURCE');
  if (typeof repository !== 'string' || !NAME.test(repository)) return no('NO_REPOSITORY');
  if (provenance.repository !== repository) return no('REPOSITORY');
  const pr = provenance.event === 'pull_request' || provenance.event === 'pull_request_target';
  // a head in another repository is a fork; a head that is not stated is treated the same (same_repository is a required switch)
  if (provenance.headRepository !== repository) return no('FORK_HEAD');
  if (!p.events[provenance.source].includes(provenance.event)) return no('EVENT');
  if (p.require_owner_actor && provenance.actorIsOwner !== true) return no('ACTOR');
  if (provenance.source === 'admin') {
    if (p.admin_requires_reachable && provenance.reachable !== true) return no('UNREACHABLE');
    return { scopes: [...SCOPES], trusted: true, why: 'TRUSTED:admin' };
  }
  const branch = provenance.headBranch;
  if (typeof branch !== 'string' || !BRANCH.test(branch)) return no('BRANCH');
  if (p.branches.some((pattern) => branchMatches(pattern, branch)))
    return { scopes: [...SCOPES], trusted: true, why: 'TRUSTED:branch' };
  // a branch of THIS repository can only be pushed by a user with write access: its pull request head is a collaborator's
  if (pr && p.collaborator_pr_heads)
    return { scopes: [...SCOPES], trusted: true, why: 'TRUSTED:collaborator_head' };
  return no('BRANCH');
}

// The scopes a token or a record carries, normalized: a missing, malformed or unknown value is ['public'] (fail closed), so a token
// issued before this field existed, or a forged shape, never gains the private scope.
export function normalizeScopes(value) {
  if (
    !Array.isArray(value) ||
    value.length < 1 ||
    value.length > SCOPES.length ||
    new Set(value).size !== value.length ||
    !value.every((s) => SCOPES.includes(s)) ||
    !value.includes('public')
  )
    return ['public'];
  return [...value];
}
export const hasPrivateScope = (value) => normalizeScopes(value).includes('private');
