import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { configuration, offerRejections } from '../src/policy.js';
import { DEFAULTS } from '../src/defaults.js';
import { parseVars } from '../deploy.mjs';
import { setup, env } from './helpers.js';
// Edits3 D (delta review, BAD_MACHINE_IDS): BAD_MACHINE_IDS keeps replacing the built-in list;
// the new BAD_MACHINE_IDS_EXTRA adds to it (or to the BAD_MACHINE_IDS override), so the owner
// can list one more machine without re-listing the defaults. Both are documented in DEPLOY.md.
// FALSIFIER: EXTRA replaces the defaults, BAD_MACHINE_IDS stops replacing them, an invalid
// EXTRA is accepted or its error is not named (or echoes the value), or the helper refuses it.
const defaults = DEFAULTS.badMachines;
const machines = (vars) => configuration({ ...env, ...vars }).badMachines;
const named = (vars) =>
  assert.throws(
    () => configuration({ ...env, ...vars }),
    (e) => e.state === 'CONFIG_VAR:BAD_MACHINE_IDS_EXTRA' && e.status === 503,
  );

test('D: without overrides the built-in list applies', () => {
  assert.ok(defaults.length > 0);
  assert.deepEqual(machines({}), defaults);
});
test('D: BAD_MACHINE_IDS still replaces the built-in list', () => {
  assert.deepEqual(machines({ BAD_MACHINE_IDS: '[5]' }), [5]);
  assert.deepEqual(machines({ BAD_MACHINE_IDS: '[]' }), []);
});
test('D: BAD_MACHINE_IDS_EXTRA adds to the built-in list', () => {
  assert.deepEqual(machines({ BAD_MACHINE_IDS_EXTRA: '[5, 6]' }), [...defaults, 5, 6]);
  // A duplicate of a default is harmless and listed once.
  assert.deepEqual(machines({ BAD_MACHINE_IDS_EXTRA: JSON.stringify([defaults[0]]) }), defaults);
});
test('D: BAD_MACHINE_IDS_EXTRA adds to a BAD_MACHINE_IDS override', () => {
  assert.deepEqual(machines({ BAD_MACHINE_IDS: '[7]', BAD_MACHINE_IDS_EXTRA: '[5]' }), [7, 5]);
});
for (const bad of ['not json', '["1"]', '[0]', '[-1]', '[1.5]', '{}', 'null', '5'])
  test('D: invalid BAD_MACHINE_IDS_EXTRA ' + JSON.stringify(bad) + ' is a named error', () => {
    named({ BAD_MACHINE_IDS_EXTRA: bad });
  });
test('D: more than 256 extra ids is a named error', () => {
  named({ BAD_MACHINE_IDS_EXTRA: JSON.stringify(Array.from({ length: 257 }, (_, i) => i + 1)) });
});
test('D: an extra id rejects its machine (MACHINE), a default id still does', () => {
  const cfg = configuration({ ...env, BAD_MACHINE_IDS_EXTRA: '[100001]' });
  assert.ok(offerRejections({ machine_id: 100001 }, cfg).includes('MACHINE'));
  assert.ok(offerRejections({ machine_id: defaults[0] }, cfg).includes('MACHINE'));
  assert.ok(!offerRejections({ machine_id: 100002 }, cfg).includes('MACHINE'));
});
test('D: a rent avoids an extra-listed machine; the error names only the variable', async () => {
  const h = await setup({ BAD_MACHINE_IDS_EXTRA: '[100001]' });
  const r = await h.call('/availability?profile=check', { method: 'GET' });
  assert.deepEqual((await r.json()).rejected, { MACHINE: 1 });
  assert.equal((await h.call()).status, 503);
  assert.equal(h.cloud.creates, 0);
  const sentinel = '[424242424242, "sentinel-value"]';
  const broken = await setup({ BAD_MACHINE_IDS_EXTRA: sentinel });
  const text = await (await broken.call()).text();
  assert.match(text, /CONFIG_VAR:BAD_MACHINE_IDS_EXTRA/);
  assert.doesNotMatch(text, /424242424242|sentinel-value/);
});
test('D: the deploy helper takes BAD_MACHINE_IDS_EXTRA; DEPLOY.md documents both', async () => {
  const vars = parseVars(
    'OWNER_GITHUB_ID=229738317\nBAD_MACHINE_IDS_EXTRA=[5]\nBAD_MACHINE_IDS=[]\n',
  );
  assert.deepEqual(
    [...vars.keys()],
    ['OWNER_GITHUB_ID', 'BAD_MACHINE_IDS_EXTRA', 'BAD_MACHINE_IDS'],
  );
  const deploy = await readFile(new URL('../DEPLOY.md', import.meta.url), 'utf8');
  assert.match(deploy, /BAD_MACHINE_IDS_EXTRA/);
  assert.match(deploy, /BAD_MACHINE_IDS\b[^.]*replac/);
});
