import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, repo, sha } from './helpers.js';
const patterns = ['**/*.md', 'docs/**', 'traces/**', 'archive/**', 'LICENSE*'];
async function policy(
  h,
  eventName = 'pull_request_target',
  event = { number: 7, pull_request: { head: { sha } } },
) {
  const { shouldRunTrigger } = await import('../trigger-policy.mjs');
  return shouldRunTrigger({
    eventName,
    event,
    repo,
    patterns,
    token: 'mock-policy-read',
    fetcher: h.cloud.fetch,
  });
}
// FALSIFIER H13: trigger eligibility is still in Worker auth, or uncertain files skip.
for (const attribute of ['base', 'paths', 'draft'])
  test('round Worker authority ignores YAML trigger attribute ' + attribute, async () => {
    const h = await setup();
    if (attribute === 'base') h.cloud.prs[0].base.ref = 'integration';
    if (attribute === 'paths') h.cloud.files = [{ filename: 'docs/only.md', status: 'modified' }];
    if (attribute === 'draft') h.cloud.prs[0].draft = true;
    const response = await h.call();
    assert.equal(response.status, 200);
    assert.equal((await response.json()).state, 'BOOTING');
    assert.equal(h.cloud.creates, 1);
  });
for (const name of ['docs', 'GPU', 'rename', 'missing', 'api-error', 'race', 'truncated'])
  test('round workflow PR path policy ' + name, async () => {
    const h = await setup();
    h.cloud.files = [{ filename: 'docs/only.md', status: 'modified' }];
    if (name === 'GPU') h.cloud.files[0].filename = 'src/gpu.rs';
    if (name === 'rename')
      Object.assign(h.cloud.files[0], { status: 'renamed', previous_filename: 'src/gpu.rs' });
    if (name === 'missing') delete h.cloud.prs[0].changed_files;
    if (name === 'api-error') h.cloud.githubError = 503;
    if (name === 'truncated') h.cloud.prs[0].changed_files = 2;
    if (name === 'race') {
      const original = h.cloud.fetch;
      let reads = 0;
      h.cloud.fetch = async (url, init) => {
        if (String(url).endsWith('/pulls/7') && ++reads === 2)
          h.cloud.prs[0].head.sha = 'e'.repeat(40);
        return original(url, init);
      };
    }
    assert.equal(await policy(h), name !== 'docs');
    assert.ok(h.cloud.calls.every((x) => x.url.startsWith('https://api.github.com/')));
  });
for (const name of ['docs', 'GPU', 'truncated', 'diverged'])
  test('round workflow push path policy ' + name, async () => {
    const h = await setup(),
      before = 'd'.repeat(40),
      after = sha;
    h.cloud.fetch = async () =>
      Response.json({
        status: name === 'diverged' ? 'diverged' : 'ahead',
        base_commit: { sha: before },
        merge_base_commit: { sha: before },
        files: Array.from({ length: name === 'truncated' ? 300 : 1 }, () => ({
          filename: name === 'GPU' ? 'src/gpu.rs' : 'docs/only.md',
          status: 'modified',
        })),
      });
    assert.equal(await policy(h, 'push', { before, after }), name !== 'docs');
  });
for (const eventName of ['workflow_dispatch', 'unknown'])
  test('round workflow policy defaults to run ' + eventName, async () => {
    assert.equal(await policy(await setup(), eventName, {}), true);
  });
