import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { setup, callbackToken } from './helpers.js';
// FALSIFIER (2026-11-11): scripts/ci/setup-cache.lock.json names a release tag that the Worker's blob
// route cannot mint: either the tag does not start with a BLOB_ALLOWED_TAG_PREFIXES entry (401) or the
// release is absent from BLOB_REPO (upstream 404, 503). The box then prints KF_SETUP_CACHE_REFUSED
// phase=locator_<kind> (scripts/ci/test_vast_setup_lock_tag.py checks that side). Local mocks only.
const lock = JSON.parse(
  readFileSync(new URL('../../../scripts/ci/setup-cache.lock.json', import.meta.url), 'utf8'),
);
const PREFIX = 'setup-cache-'; // docs/design/VAST_SETUP_CACHE_20261023.md: the prefix the Worker var must list
const signed =
  'https://release-assets.githubusercontent.com/github-production-release-asset/9/blob?sig=x&se=2026-11-12';
async function harness({ releases, prefixes = JSON.stringify([PREFIX]) }) {
  const h = await setup({
    BLOB_REPO: 'owner/private-store',
    BLOB_ALLOWED_TAG_PREFIXES: prefixes,
    BLOB_GITHUB_TOKEN: 'mock-blob-read-secret',
  });
  await h.call();
  const base = h.cloud.fetch;
  const calls = [];
  h.cloud.fetch = async (url, init = {}) => {
    const u = String(url);
    if (!u.includes('/repos/owner/private-store/')) return base(url, init);
    calls.push(u);
    const tag = /\/releases\/tags\/(.+)$/.exec(u)?.[1];
    if (tag) {
      const assets = releases[tag];
      return assets ? Response.json({ assets }) : new Response('', { status: 404 });
    }
    return new Response(null, { status: 302, headers: { Location: signed } });
  };
  h.restart();
  const token = callbackToken(await h.storage.get('record:123-1'));
  return {
    calls,
    get: (tag, name) =>
      h.service.fetch(
        new Request(h.bindings.WORKER_URL + '/blob-url/123-1/' + tag + '/' + name, {
          headers: { Authorization: 'Bearer ' + token },
        }),
      ),
  };
}
const listing = (tag) => ({
  [tag]: lock.assets.map((a, i) => ({ id: 100 + i, name: a.name, size: a.size })),
});

test('the lock tag carries the documented allowed prefix', () => {
  assert.ok(lock.tag.startsWith(PREFIX), lock.tag);
  assert.ok(lock.assets.length >= 2);
});
for (const asset of lock.assets)
  test('a lock tag matching the allowed prefix resolves: ' + asset.kind, async () => {
    const x = await harness({ releases: listing(lock.tag) });
    const r = await x.get(lock.tag, asset.name);
    assert.equal(r.status, 200, await r.clone().text());
    const d = await r.json();
    assert.equal(d.name, asset.name);
    assert.equal(d.size, asset.size);
    assert.equal(d.url, signed);
    assert.ok(x.calls[0].endsWith('/releases/tags/' + lock.tag));
  });
test('a lock tag absent from BLOB_REPO is refused after one metadata lookup', async () => {
  const x = await harness({ releases: listing(PREFIX + 'some-other-tag-v1') });
  for (const asset of lock.assets) {
    const r = await x.get(lock.tag, asset.name);
    assert.equal(r.status, 503);
  }
  assert.ok(x.calls.length >= 1 && x.calls.every((u) => u.includes('/releases/tags/' + lock.tag)));
});
test('a release whose assets lack the locked name is refused', async () => {
  const x = await harness({ releases: { [lock.tag]: [{ id: 5, name: 'other.tar.gz', size: 1 }] } });
  assert.equal((await x.get(lock.tag, lock.assets[0].name)).status, 503);
});
test('a lock tag outside the allowed prefixes is refused before any upstream call', async () => {
  const x = await harness({
    releases: listing(lock.tag),
    prefixes: JSON.stringify(['ci-image-']),
  });
  assert.equal((await x.get(lock.tag, lock.assets[0].name)).status, 401);
  assert.equal(x.calls.length, 0);
});
