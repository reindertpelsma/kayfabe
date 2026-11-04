// FALSIFIER: the committed status fixture is stale, unlabelled, or carries a deployment URL.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { buildFixture, serialize, FIXTURE_PATH } from '../scripts/make-fixture.mjs';

test('site/status/fixture.json equals a fresh synthetic build', async () => {
  const committed = readFileSync(FIXTURE_PATH, 'utf8');
  assert.equal(committed, serialize(await buildFixture()), 'run: npm run fixture -- --write');
});

test('the fixture is labelled synthetic and holds no URL', () => {
  const text = readFileSync(FIXTURE_PATH, 'utf8');
  const fixture = JSON.parse(text);
  assert.equal(fixture.synthetic, true);
  assert.match(fixture.note, /SYNTHETIC/);
  assert.doesNotMatch(text, /https?:\/\/|kayfabe\.dev|192\.0\.2\.|synthetic-tag-/);
  for (const body of Object.values(fixture.responses))
    for (const run of body.runs ?? []) assert.match(run.id, /^syn-/);
  assert.ok(Object.keys(fixture.responses).length > 100);
});
