import test from 'node:test';
import assert from 'node:assert/strict';
import { ALLOWED } from '../deploy.mjs';

// MEASURED 2026-10-26: a re-run of the same commit answered UNAVAILABLE_BUDGET for 24 h (the SHA gap),
// and deploy.mjs refused to set SHA_GAP_HOURS ("not an allowed public var"), so a required gate could
// not be retried after an infrastructure failure. FALSIFIER: SHA_GAP_HOURS is not an allowed public var.
test('SHA_GAP_HOURS is an allowed public var', () => {
  assert.ok(ALLOWED.has('SHA_GAP_HOURS'));
});
test('the allow-list still excludes secrets and unknown names', () => {
  for (const name of ['VAST_API_KEY', 'RUN_TOKEN_SECRET', 'OWNER_STOP_TOKEN', 'BOX_ACCESS_SECRET'])
    assert.ok(!ALLOWED.has(name), name);
});
