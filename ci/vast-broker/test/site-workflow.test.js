import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { checkWorkflows } from '../workflow-guard.js';

const gpu = await readFile(
  new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
  'utf8',
);
const site = `name: Site
on:
  pull_request:
    paths: ['site/**', 'docs/**']
permissions: {}
jobs:
  build:
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
  deploy:
    needs: build
    if: >-
      \${{ github.event_name != 'pull_request' &&
          (github.ref == 'refs/heads/master' ||
           startsWith(github.ref, 'refs/tags/v')) }}
    permissions:
      pages: write
      id-token: write
    environment:
      name: github-pages
    steps:
      - uses: actions/deploy-pages@368f82528645a54fb793d4d04e342629a3f51346
`;
const check = (text, name = 'site.yml') => checkWorkflows({ 'gpu-vast.yml': gpu, [name]: text });

// FALSIFIER: the Pages exception grants OIDC outside this exact build/deploy boundary.
test('site workflow permits only its reviewed Pages deploy boundary', () => {
  assert.deepEqual(check(site), []);
});
for (const extra of ['contents: write', 'actions: write'])
  test('site deploy rejects extra permission ' + extra, () => {
    assert.ok(
      check(site.replace('      pages: write', '      pages: write\n      ' + extra)).length,
    );
  });
for (const defect of ['condition', 'environment', 'dependency'])
  test('site deploy fails closed without ' + defect, () => {
    const changed =
      defect === 'condition'
        ? site.replace("github.event_name != 'pull_request'", 'true')
        : defect === 'environment'
          ? site.replace('name: github-pages', 'name: other')
          : site.replace('needs: build', 'needs: other');
    assert.ok(check(changed).length);
  });
for (const name of ['other.yml', 'pages.yml'])
  test('Pages OIDC exception does not apply to ' + name, () => {
    assert.ok(check(site, name).length);
  });
for (const permission of ['id-token: write', 'pages: write'])
  test('site build cannot gain ' + permission, () => {
    assert.ok(check(site.replace('      contents: read', '      ' + permission)).length);
  });
for (const name of ['ordinary.yml', 'publish.yml'])
  test('unrelated job cannot acquire OIDC ' + name, () => {
    assert.ok(check(`jobs:\n  build:\n    permissions:\n      id-token: write\n`, name).length);
  });
test('site actions must be SHA pinned', () => {
  assert.ok(
    check(
      site.replace(
        'actions/checkout@11d5960a326750d5838078e36cf38b85af677262',
        'actions/checkout@main',
      ),
    ).length,
  );
});
