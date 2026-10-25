import test from 'node:test';
import assert from 'node:assert/strict';
import { createReaper } from '../../vast-reaper/worker.js';

// The Workers runtime rejects redirect: 'error' ("won't be implemented since it does not make
// sense at the edge"), so every reaper tick threw before the list request and logged
// reaper_unavailable/transport. This fetcher refuses the same values the runtime refuses.
const env = {
  REAPER_VAST_API_KEY: 'reaper-mock-key',
  REAPER_HARD_MINUTES: '180',
  REAPER_MARGIN_MINUTES: '15',
};
const edgeFetcher = (respond, seen) => async (input, init = {}) => {
  if (init.redirect !== 'follow' && init.redirect !== 'manual' && init.redirect !== undefined)
    throw new TypeError('Invalid redirect value, must be one of "follow" or "manual"');
  seen.push({ url: String(input), redirect: init.redirect, method: init.method });
  return respond(new URL(input));
};

test('list request uses a redirect mode the Workers runtime accepts', async () => {
  const seen = [],
    logs = [];
  const worker = createReaper({
    now: () => 1800000000000,
    report: (x) => logs.push(x),
    fetcher: edgeFetcher(() => Response.json({ success: true, instances: [] }), seen),
  });
  await worker.sweep(env);
  assert.deepEqual(
    seen.map((x) => x.redirect),
    ['manual'],
  );
  assert.equal(logs.at(-1).event, 'reaper_complete');
});

test('a redirect from the provider is refused, never followed', async () => {
  const seen = [],
    logs = [];
  const worker = createReaper({
    now: () => 1800000000000,
    report: (x) => logs.push(x),
    fetcher: edgeFetcher(
      () => new Response(null, { status: 302, headers: { location: 'https://evil.example/' } }),
      seen,
    ),
  });
  await worker.sweep(env);
  assert.equal(seen.length, 1);
  assert.deepEqual(logs.at(-1), { event: 'reaper_unavailable', reason: 'http_refusal' });
});
