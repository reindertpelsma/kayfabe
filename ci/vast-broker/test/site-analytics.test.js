import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const site = readFileSync(new URL('../../../.github/workflows/site.yml', import.meta.url), 'utf8');
const block = (id) => site.split('  ' + id + ':\n')[1]?.split(/^  [a-z][a-z-]*:/m)[0] ?? '';

test('analytics variable is scoped only to the Pages deploy job', () => {
  assert.doesNotMatch(block('policy') + block('build'), /WEB_ANALYTICS_SNIPPET|vars\./);
  assert.match(block('deploy'), /WEB_ANALYTICS_SNIPPET: \$\{\{ vars.WEB_ANALYTICS_SNIPPET \}\}/);
});
test('ordinary render and output tests run before the uninstrumented upload', () => {
  assert.match(block('build'), /python3 site\/test_output.py/);
  assert.doesNotMatch(block('build'), /deploy_analytics\.py/);
  assert.match(block('deploy'), /name: github-pages-deploy/);
  assert.match(block('deploy'), /artifact_name: github-pages-deploy/);
});
test('analytics transformer uses the exact build tooling revision without checkout', () => {
  assert.match(block('build'), /tooling_ref: \$\{\{ steps.tooling.outputs.sha \}\}/);
  assert.match(block('deploy'), /TOOLING_REF: \$\{\{ needs.build.outputs.tooling_ref \}\}/);
  assert.match(block('deploy'), /\$source_origin\/\$TOOLING_REF\/site\/deploy_analytics.py/);
  assert.doesNotMatch(block('deploy'), /actions\/checkout|contents:|actions: write/);
});
test('deploy transformation uses runtime HTML as data, preserving environment and policy guards', () => {
  assert.match(block('deploy'), /SITE_DEPLOY_ENVIRONMENT: github-pages/);
  assert.match(block('deploy'), /needs.policy.outputs.deploy == '1'/);
  assert.match(block('deploy'), /github.event_name != 'pull_request'/);
  assert.match(block('deploy'), /python3 -I .*deploy_analytics.py/);
  assert.doesNotMatch(site, /run:[\s\S]*\$\{\{ vars.WEB_ANALYTICS_SNIPPET \}\}[^\n]*\n\s*[^e]/);
});
