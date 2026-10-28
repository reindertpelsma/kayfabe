import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const site = readFileSync(new URL('../../../.github/workflows/site.yml', import.meta.url), 'utf8');
const block = (id) => site.split('  ' + id + ':\n')[1]?.split(/^  [a-z][a-z-]*:/m)[0] ?? '';

test('PR build has no environment and no analytics variable reference', () => {
  assert.doesNotMatch(block('build'), /environment:|WEB_ANALYTICS_SNIPPET|vars\./);
  assert.match(block('deploy'), /WEB_ANALYTICS_SNIPPET: \$\{\{ vars.WEB_ANALYTICS_SNIPPET \}\}/);
});
test('deploy executes only fixed inline Python and no downloaded or checkout helper', () => {
  assert.match(block('deploy'), /python3 -I - <<'PY'/);
  assert.doesNotMatch(block('deploy'), /checkout|curl|TOOLING_REF|deploy_analytics\.py/);
});
test('privileged deploy cannot execute repository scripts', () => {
  const commands = block('deploy').split('        run: |\n')[1] ?? '';
  assert.match(commands.trimStart(), /^python3 -I - <<'PY'/);
  assert.doesNotMatch(block('deploy'), /python3 -I ["$\/]|git rev-parse|source_origin/);
});
test('empty variable skips download, patch and upload', () => {
  const steps = block('deploy').split('      - ').slice(1);
  for (const step of steps.filter(s => !s.includes('actions/deploy-pages@'))) {
    assert.match(step, /if:.*vars.WEB_ANALYTICS_SNIPPET != ''/);
  }
  assert.match(block('deploy'), /artifact_name:.*steps.analytics.outputs.artifact_name.*github-pages/);
});
test('only the transformed directory gets the separate deploy artifact', () => {
  assert.match(block('deploy'), /actions\/upload-pages-artifact@[0-9a-f]{40}/);
  assert.match(block('deploy'), /name: github-pages-deploy/);
  assert.match(block('deploy'), /path:.*pages-patched/);
  assert.match(block('deploy'), /steps.analytics.outputs.artifact_name == 'github-pages-deploy'/);
});
test('original Pages permission and policy separation remain intact', () => {
  assert.match(block('deploy'), /needs.policy.outputs.deploy == '1'/);
  assert.match(block('deploy'), /github.event_name != 'pull_request'/);
  assert.match(block('deploy'), /SITE_DEPLOY_ENVIRONMENT: github-pages/);
  assert.doesNotMatch(block('deploy'), /contents:|actions: write/);
  assert.doesNotMatch(block('build'), /id-token:|pages:/);
});
