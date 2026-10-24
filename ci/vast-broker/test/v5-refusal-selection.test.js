import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, sha } from './helpers.js';
// FALSIFIER: a read-only budget refusal cannot demonstrate the forced choice,
// or its public metadata exposes provider identifiers rather than local enums.
for (const denied of ['UNKNOWN', 'AD107'])
  test('V5 budget refusal reports forced preference ' + denied, async () => {
    const h = await setup({ DAILY_SPEND_USD: '0.00001' }),
      r = await h.call('/rent', {
        body: {
          runner_nonce: 'f'.repeat(64),
          tested_sha: sha,
          profile: 'check',
          remaining_seconds: 12000,
          preferences: [denied, 'AD104'],
        },
      });
    assert.equal(r.status, 409);
    const data = await r.json();
    assert.deepEqual(data.selection, { die: 'AD104', ignored: 1, reason: 'PREFERENCE' });
    assert.equal(h.cloud.creates, 0);
    assert.doesNotMatch(JSON.stringify(data), /machine_id|public_ip|offer_id/);
  });
