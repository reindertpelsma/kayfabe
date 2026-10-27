import test from 'node:test';
import assert from 'node:assert/strict';
import { Vast } from '../src/vast.js';

// MEASURED 2026-10-25 live: every create failed with class 'transport' in 600 ms. The Workers
// runtime throws "Illegal invocation" when the global fetch is called as a method of another
// object (`this.fetcher(...)` with this = the Vast instance); Node ignores `this`, so the mocks
// could not see it. This fetcher refuses what the runtime refuses.
// FALSIFIER: create() succeeds against a fetcher that rejects a method-style call.
const strict = (respond) =>
  function (input, init) {
    if (this !== undefined && this !== globalThis) throw new TypeError('Illegal invocation');
    return respond(input, init);
  };
const cfg = { price: 1, sweepPrice: 1, maxInet: 0.02 };

test('Vast.create does not call the injected fetcher as a method', async () => {
  const vast = new Vast(
    strict(() => Response.json({ success: true, new_contract: 4242 })),
    cfg,
    'k',
  );
  const id = await vast.create({ id: 7 }, { label: 'kf-ci-1-1' }, '#!/bin/sh\n', {});
  assert.equal(id, '4242');
});

test('Vast.api does not call the injected fetcher as a method', async () => {
  const vast = new Vast(
    strict(() => Response.json({ success: true, credit: 12 })),
    cfg,
    'k',
  );
  assert.equal(await vast.credit(), 12);
});
