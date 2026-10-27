import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, existsSync } from 'node:fs';
import { checkWorkflows } from '../workflow-guard.js';
const read = (file) =>
  readFileSync(new URL('../../../.github/workflows/' + file, import.meta.url), 'utf8');
const gpu = read('gpu-vast.yml');
const site = read('site.yml');
const statusPath = new URL('../../../.github/workflows/site-status.yml', import.meta.url);
const status = existsSync(statusPath) ? readFileSync(statusPath, 'utf8') : '';
const check = (s = site, r = status) =>
  checkWorkflows({ 'gpu-vast.yml': gpu, 'site.yml': s, 'site-status.yml': r });
const block = (id) => site.split('  ' + id + ':\n')[1]?.split(/^  [a-z][a-z-]*:/m)[0] ?? '';
for (const label of ['docs', 'code'])
  test('site always covers PRs and decides before building ' + label, () => {
    assert.doesNotMatch(site.split('  push:')[0], /paths:/);
    assert.match(block('policy'), /pull-requests: read/);
    assert.doesNotMatch(block('policy'), /checkout|statuses: write|id-token: write/);
    assert.match(block('build'), /needs: policy/);
    assert.match(block('build'), /needs.policy.outputs.run == '1'/);
    assert.match(block('build'), /github.event.pull_request.head.sha/);
  });
for (const label of ['normal push', 'tag keeps guarded deploy'])
  test('site deploy requires path authorization ' + label, () => {
    assert.match(block('deploy'), /needs: \[policy, build\]/);
    assert.match(block('deploy'), /needs.policy.outputs.deploy == '1'/);
    assert.match(block('deploy'), /github.event_name != 'pull_request'/);
  });
for (const label of ['fork start', 'fork completion'])
  test('site statuses are API-only with trusted triggers ' + label, () => {
    assert.match(status, /pull_request_target:/);
    assert.match(status, /workflow_run:/);
    assert.match(status, /workflows: \[Site\]/);
    assert.match(status, /statuses: write/);
    assert.doesNotMatch(status, /checkout|artifact|cache@|secrets\.|id-token:|pages:/);
  });
test('site new boundaries are accepted', () => assert.deepEqual(check(), []));
for (const grant of ['contents', 'pull-requests'])
  test('site policy read-only boundary ' + grant, () => {
    assert.ok(
      check(site.replace('      ' + grant + ': read', '      ' + grant + ': write')).length,
    );
  });
for (const grant of ['contents', 'actions'])
  test('site status writer cannot gain write ' + grant, () => {
    assert.ok(
      check(site, status.replace('      ' + grant + ': read', '      ' + grant + ': write')).length,
    );
  });
for (const command of ['evil.py', 'runner.py'])
  test('site status writer cannot execute another source ' + command, () => {
    assert.ok(check(site, status.replaceAll('site_policy.py', command)).length);
  });
for (const paths of ['site/**', 'docs/**'])
  test('site required status cannot regain PR path filter ' + paths, () => {
    assert.ok(
      check(site.replace('  pull_request:\n', "  pull_request:\n    paths: ['" + paths + "']\n"))
        .length,
    );
  });
