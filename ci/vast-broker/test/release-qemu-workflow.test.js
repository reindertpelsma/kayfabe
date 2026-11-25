import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const root = new URL('../../../', import.meta.url);
const file = new URL('.github/workflows/release-qemu.yml', root);
const guard = () => import('../workflow-guard.js');
const read = (name) => readFile(new URL('.github/workflows/' + name, root), 'utf8');
const refused = (errors) => errors.some((e) => e.includes('OIDC outside approved'));
const OIDC = '      id-token: write\n';
const withOidc = (text, jobHeader) => {
  const moved = text.replace(jobHeader, jobHeader + OIDC);
  assert.notEqual(moved, text);
  return moved;
};
test('the QEMU release workflow passes the repository workflow boundaries', async () => {
  const { checkRepository } = await guard();
  assert.deepEqual(await checkRepository(root), []);
});
test('OIDC is allowed in the attest job of the QEMU release workflow', async () => {
  const { checkWorkflows } = await guard();
  const text = await readFile(file, 'utf8');
  assert.match(text, /^  attest:\n(?:.*\n)*?    permissions:\n      id-token: write\n/m);
  assert.deepEqual(
    checkWorkflows({ 'gpu-vast.yml': await read('gpu-vast.yml'), 'release-qemu.yml': text }),
    [],
  );
});
test('the build job of the QEMU release workflow is refused OIDC', async () => {
  const { checkWorkflows } = await guard();
  const text = await readFile(file, 'utf8');
  const moved = text.replace(
    '    permissions:\n      contents: read\n    outputs:',
    '    permissions:\n      contents: read\n      id-token: write\n    outputs:',
  );
  assert.notEqual(moved, text);
  assert.ok(
    refused(
      checkWorkflows({ 'gpu-vast.yml': await read('gpu-vast.yml'), 'release-qemu.yml': moved }),
    ),
  );
});
for (const [name, header] of [
  ['draft', '    permissions:\n      contents: write\n'],
  ['gate', '    permissions:\n      contents: read\n      statuses: read\n'],
])
  test('the ' + name + ' job of the QEMU release workflow is refused OIDC', async () => {
    const { checkWorkflows } = await guard();
    const text = await readFile(file, 'utf8');
    const moved = withOidc(text, header);
    assert.ok(
      refused(
        checkWorkflows({ 'gpu-vast.yml': await read('gpu-vast.yml'), 'release-qemu.yml': moved }),
      ),
    );
  });
test('workflow-level OIDC in the QEMU release workflow is refused', async () => {
  const { checkWorkflows } = await guard();
  const text = await readFile(file, 'utf8');
  const moved = text.replace('permissions: {}\n', 'permissions:\n  id-token: write\n');
  assert.notEqual(moved, text);
  assert.ok(
    checkWorkflows({ 'gpu-vast.yml': await read('gpu-vast.yml'), 'release-qemu.yml': moved }).some(
      (e) => e.includes('workflow-level OIDC'),
    ),
  );
});
test('an attest job in another file, or a differently named file, is refused OIDC', async () => {
  const { checkWorkflows } = await guard();
  const text = await readFile(file, 'utf8');
  const gpu = await read('gpu-vast.yml');
  assert.ok(refused(checkWorkflows({ 'gpu-vast.yml': gpu, 'release-qemu.yaml': text })));
  assert.ok(refused(checkWorkflows({ 'gpu-vast.yml': gpu, 'other.yml': text })));
});
test('the attest job is matched by its exact name', async () => {
  const { checkWorkflows } = await guard();
  const gpu = await read('gpu-vast.yml');
  const text = await readFile(file, 'utf8');
  const renamed = text.replace(/^  attest:$/m, '  attest-binaries:');
  assert.notEqual(renamed, text);
  assert.ok(refused(checkWorkflows({ 'gpu-vast.yml': gpu, 'release-qemu.yml': renamed })));
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
