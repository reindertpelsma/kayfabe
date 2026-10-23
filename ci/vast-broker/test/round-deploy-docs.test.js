import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
// FALSIFIER V11/V12/V2/V4 docs: the deploy recipe cannot be followed as written, states a
// schedule its toml does not carry, or keeps instructions for retired features.
const text = (path) => readFile(new URL(path, import.meta.url), 'utf8');
const deploy = () => text('../DEPLOY.md');
// Prose checks ignore line wrapping and Markdown quote markers.
const prose = async () => (await deploy()).replace(/\n> ?/g, '\n').replace(/\s+/g, ' ');
const reaperSection = async () => (await deploy()).split('## Independent reaper')[1] ?? '';
const crons = async (path) => /^crons = \["([^"]+)"\]$/m.exec(await text(path))?.[1];
test('round DEPLOY test commands work from both documented directories', async () => {
  const doc = await prose();
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
test('round main Worker keeps workers.dev but no preview hostnames', async () => {
  assert.match(
    await text('../wrangler.toml'),
    /^workers_dev = true\n(?:#.*\n)*preview_urls = false$/m,
  );
});
test('round reaper ticks every five minutes for the 195-minute horizon', async () => {
  assert.equal(await crons('../../vast-reaper/wrangler.toml'), '*/5 * * * *');
});
const POLICY =
  'Worker authorizes: signed identity + pinned workflow + triggering_actor == owner + ' +
  'sha == run head. YAML decides when to run.';
test('round DEPLOY states the owner authorization sentence', async () => {
  assert.ok((await prose()).includes(POLICY));
});
test('round DEPLOY policy section opens with the sentence and the trigger warning', async () => {
  const section = (await prose()).split('## Authorization policy')[1].split('## ')[0];
  assert.ok(section.indexOf(POLICY) >= 0 && section.indexOf(POLICY) < 300);
  assert.match(section, /Reopen, Ready for review or Update branch/);
  assert.match(section, /owner-applied label/);
});
test('round DEPLOY documents the helper and the first-rental acceptance checks', async () => {
  const doc = await prose();
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
test('round DEPLOY V2 acceptance check names every instance price field', async () => {
  const section = (await prose()).split('## V2 acceptance check')[1]?.split('## ')[0] ?? '';
  for (const field of ['dph_base', 'dph_total', 'storage_cost', 'inet_up_cost', 'inet_down_cost'])
    assert.ok(section.includes('`' + field + '`'), field);
});
