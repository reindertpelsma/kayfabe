import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const root = new URL('../../../', import.meta.url);
const file = new URL('.github/workflows/release-qemu.yml', root);
const guard = () => import('../workflow-guard.js');
test('the QEMU release workflow passes the repository workflow boundaries', async () => {
  const { checkRepository } = await guard();
  assert.deepEqual(await checkRepository(root), []);
});
test('OIDC in the QEMU release workflow is allowed in the build job only', async () => {
  const { checkWorkflows } = await guard();
  const gpu = await readFile(new URL('.github/workflows/gpu-vast.yml', root), 'utf8');
  const text = await readFile(file, 'utf8');
  assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': gpu, 'release-qemu.yml': text }), []);
  const moved = text.replace(
    '    timeout-minutes: 15\n    permissions:\n      contents: write',
    '    timeout-minutes: 15\n    permissions:\n      contents: write\n      id-token: write',
  );
  assert.notEqual(moved, text);
  assert.ok(
    checkWorkflows({ 'gpu-vast.yml': gpu, 'release-qemu.yml': moved }).some((e) =>
      e.includes('OIDC outside approved'),
    ),
  );
});
test('the QEMU release workflow is dispatch only, draft only, with pinned actions', async () => {
  const t = await readFile(file, 'utf8');
  assert.match(t, /^on:\n  workflow_dispatch:/m);
  assert.doesNotMatch(t, /^  (push|pull_request|pull_request_target|schedule|workflow_run):/m);
  assert.match(t, /gh release create .* --draft /);
  assert.doesNotMatch(t, /gh release edit|--draft=false|secrets\./);
  for (const ref of t.matchAll(/^\s*(?:- )?uses: (\S+)/gm))
    assert.match(ref[1], /^[\w.-]+\/[\w./-]+@[0-9a-f]{40}$/);
});
