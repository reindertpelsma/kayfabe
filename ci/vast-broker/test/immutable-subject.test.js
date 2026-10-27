import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setup } from './helpers.js';
// MEASURED 2026-10-24 (Worker log 'identity:sub'): the repository uses GitHub's immutable OIDC
// subject, repo:<owner>@<owner id>/<repo>@<repo id>:environment:<name>. The configured OIDC_SUB
// must be accepted in that shape and refuse the mutable name-only form.
// FALSIFIER: the immutable form is refused as a setting or as a token, or the old form passes.
const immutable = 'repo:reindertpelsma@229738317/kayfabe@1311106387:environment:gpu-vast';
const mutable = 'repo:reindertpelsma/kayfabe:environment:gpu-vast';
test('an immutable-form OIDC_SUB accepts a token with that subject', async () => {
  const h = await setup({ OIDC_SUB: immutable });
  const r = await h.call('/availability?profile=check', {
    method: 'GET',
    claims: { sub: immutable },
  });
  assert.notEqual(r.status, 401);
  assert.notEqual(r.status, 503);
});
test('an immutable-form OIDC_SUB refuses the mutable subject', async () => {
  const h = await setup({ OIDC_SUB: immutable });
  const r = await h.call('/availability?profile=check', {
    method: 'GET',
    claims: { sub: mutable },
  });
  assert.equal(r.status, 401);
});
test('wrangler.toml pins the immutable subject of this repository', () => {
  const toml = readFileSync(new URL('../wrangler.toml', import.meta.url), 'utf8');
  assert.match(toml, new RegExp('^OIDC_SUB = "' + immutable + '"$', 'm'));
});
