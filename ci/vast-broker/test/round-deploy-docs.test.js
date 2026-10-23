import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
// FALSIFIER V11/V12/V2/V4 docs: the deploy recipe cannot be followed as written, states a
// schedule its toml does not carry, or keeps instructions for retired features.
const text = (path) => readFile(new URL(path, import.meta.url), 'utf8');
const deploy = () => text('../DEPLOY.md');
const reaperSection = async () => (await deploy()).split('## Independent reaper')[1] ?? '';
const crons = async (path) => /^crons = \["([^"]+)"\]$/m.exec(await text(path))?.[1];
test('round DEPLOY test commands work from both documented directories', async () => {
  const doc = await deploy();
  assert.match(doc, /In `ci\/vast-broker`, run `npm test`/);
  assert.match(doc, /from the repository root, run `node --test ci\/vast-broker\/test`/);
});
test('round DEPLOY reaper recipe passes no main-Worker var', async () => {
  assert.doesNotMatch(await reaperSection(), /--var/);
});
test('round DEPLOY reaper secret is put only after the first deploy', async () => {
  const section = await reaperSection();
  const deployAt = section.indexOf('wrangler deploy\n');
  const secretAt = section.indexOf('wrangler secret put REAPER_VAST_API_KEY');
  assert.ok(deployAt > 0 && secretAt > deployAt, 'deploy precedes secret put');
});
for (const [label, toml, phrase] of [
  ['reaper', '../../vast-reaper/wrangler.toml', 'Reaper cron schedule'],
  ['main', '../wrangler.toml', 'Main Worker cron schedule'],
])
  test('round DEPLOY states the toml cron schedule ' + label, async () => {
    const schedule = await crons(toml);
    assert.ok(schedule);
    assert.ok((await deploy()).includes(phrase + ': `' + schedule + '`'));
  });
test('round main Worker disables preview URLs', async () => {
  assert.match(await text('../wrangler.toml'), /^preview_urls = false$/m);
});
test('round DEPLOY states the authorization policy and its owner-trigger warning', async () => {
  const doc = await deploy();
  assert.ok(
    doc.includes(
      'Worker authorizes: signed identity + pinned workflow + triggering_actor == owner + ' +
        'sha == run head. YAML decides when to run.',
    ),
  );
  assert.match(doc, /Reopen, Ready for review or Update branch/);
  assert.match(doc, /owner-applied label/);
});
test('round DEPLOY documents the helper and the first-rental acceptance checks', async () => {
  const doc = await deploy();
  assert.match(doc, /node ci\/vast-broker\/deploy\.mjs \/path\/outside\/checkout\/broker\.vars/);
  assert.match(doc, /V2 acceptance check/);
  assert.match(doc, /dashboard-only/);
});
for (const path of ['../DEPLOY.md', '../README.md'])
  test('round docs keep no live instruction for retired features ' + path, async () => {
    const doc = await text(path);
    assert.doesNotMatch(doc, /REPLACE_ME|--var OWNER_GITHUB_ID|ACCESS_RUNNER_CLIENT_ID=/);
    for (const line of doc.split('\n').filter((x) => /stop-timer|\/hold|APPROVAL_MODE/.test(x)))
      assert.match(line, /SUPERSEDED|retired|removed/i, line.slice(0, 80));
  });
