import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
// FALSIFIER: shipped numeric bindings silently override the owner-approved
// single policy file, or legacy lifetime bindings remain in deployment config.
for (const [name, keys] of [
  ['money', ['CREDIT_RESERVE_USD', 'MAX_PRICE_USD_HOUR', 'DAILY_SPEND_USD', 'ROLLING_SPEND_USD']],
  ['clocks', ['TTL_MINUTES', 'PROVISION_MINUTES', 'POST_RESULT_GRACE_MINUTES']],
])
  test('V5 shipped ' + name + ' derives from one policy file', async () => {
    const text = await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8');
    for (const key of keys) assert.doesNotMatch(text, new RegExp('^' + key + '\\s*=', 'm'));
    assert.match(text, /RENTING_ENABLED = "false"/);
  });
