import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const site = readFileSync(new URL('../../../.github/workflows/site.yml', import.meta.url), 'utf8');
const block = (id) => site.split('  ' + id + ':\n')[1]?.split(/^  [a-z][a-z-]*:/m)[0] ?? '';

test('PR build has no environment and no analytics variable reference', () => {
  assert.doesNotMatch(block('build'), /environment:|WEB_ANALYTICS_SNIPPET|vars\./);
  assert.match(block('build'), /--deploy-candidate/);
});
test('deploy executes only fixed inline Python and no helper or page parser', () => {
  assert.match(block('deploy'), /python3 -I - <<'PY'/);
  assert.doesNotMatch(block('deploy'), /checkout|curl|TOOLING_REF|deploy_analytics\.py/);
  assert.doesNotMatch(block('deploy'), /getpos|Tokens\(text\)|Tokens\(changed|positions/);
});
test('deploy only tokenizes the variable and replaces neutral slot bytes', () => {
  assert.match(block('deploy'), /Tokens\(snippet\)/);
  assert.match(block('deploy'), /kf-slot-a/);
  assert.match(block('deploy'), /\.replace\(slot, replacement\)/);
  assert.doesNotMatch(block('deploy'), /\.decode\("utf-8"\)|write_text/);
});
test('absent variable still strips the slot and republishes clean output', () => {
  assert.doesNotMatch(block('deploy'), /vars.WEB_ANALYTICS_SNIPPET != ''/);
  assert.match(block('deploy'), /artifact_name: github-pages-deploy/);
  assert.match(block('deploy'), /actions\/upload-pages-artifact@[0-9a-f]{40}/);
});
test('original permissions and policy separation remain intact', () => {
  assert.match(block('deploy'), /needs.policy.outputs.deploy == '1'/);
  assert.match(block('deploy'), /github.event_name != 'pull_request'/);
  assert.doesNotMatch(block('deploy'), /contents:|actions: write/);
  assert.doesNotMatch(block('build'), /id-token:|pages:/);
});
