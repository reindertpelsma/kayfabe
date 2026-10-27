import test from 'node:test';
import assert from 'node:assert/strict';
import { createReaper } from '../../vast-reaper/worker.js';

// FALSIFIER: any failure loses its static reason, exposes supplied text, issues a retry,
// or deletes a row when either pre-delete snapshot fails. Fixtures contain synthetic data only.
const poison = 'synthetic-sensitive-marker';
const now = 1800000000000;
const old = { id: 1, label: 'kf-ci-1-1', start_date: (now - 999 * 60000) / 1000 };
const listed = (extra = {}) => Response.json({ success: true, instances: [old], ...extra });
const cases = [
  ['configuration', 'missing key', { REAPER_VAST_API_KEY: undefined }],
  ['configuration', 'invalid margin', { REAPER_MARGIN_MINUTES: poison }],
  [
    'transport',
    'error text',
    {},
    () => {
      throw Error(poison);
    },
  ],
  [
    'transport',
    'arbitrary thrown object',
    {},
    () => {
      throw { message: poison, stack: poison, reason: poison, headers: { key: poison } };
    },
  ],
  ['http_refusal', 'forbidden', {}, () => new Response(poison, { status: 403 })],
  ['http_refusal', 'server error', {}, () => new Response(poison, { status: 503 })],
  ['invalid_json', 'invalid syntax', {}, () => new Response(poison)],
  ['invalid_json', 'invalid encoding', {}, () => new Response(new Uint8Array([255]))],
  ['invalid_response', 'missing body', {}, () => new Response(null)],
  ['invalid_response', 'oversized body', {}, () => new Response('x'.repeat(262145))],
  ['incomplete_list', 'continuation token', {}, () => listed({ next_token: poison })],
  ['incomplete_list', 'continuation link', {}, () => listed({ next: poison })],
  ['incomplete_list', 'row cap', {}, () => listed({ instances: Array(100).fill(old) })],
  ['incomplete_list', 'provider non-success', {}, () => listed({ success: false, error: poison })],
  ['incomplete_list', 'missing instances', {}, () => Response.json({ success: true })],
  ['malformed_list', 'duplicate id', {}, () => listed({ instances: [old, old] })],
  ['malformed_list', 'invalid id', {}, () => listed({ instances: [old, { id: poison }] })],
  ['malformed_list', 'null document', {}, () => Response.json(null)],
  [
    'transport',
    'stream error',
    {},
    () =>
      new Response(
        new ReadableStream({
          start(controller) {
            controller.error(Error(poison));
          },
        }),
      ),
  ],
  ['transport', 'rejected promise', {}, () => Promise.reject(poison)],
];
for (const [reason, name, overrides, fault] of cases) {
  for (const snapshot of reason === 'configuration' ? [0] : [1, 2]) {
    test(`reaper diagnostic ${reason}: ${name}, snapshot ${snapshot}`, async () => {
      const logs = [],
        calls = [];
      const worker = createReaper({
        now: () => now,
        report: (event) => logs.push(event),
        fetcher: async (_input, init) => {
          calls.push(init.method);
          return calls.length === snapshot ? fault() : listed();
        },
      });
      await worker.sweep({ REAPER_VAST_API_KEY: poison, ...overrides });
      assert.deepEqual(calls, Array(snapshot).fill('GET'), 'no DELETE or retry');
      assert.deepEqual(logs, [{ event: 'reaper_unavailable', reason }]);
      assert.equal(JSON.stringify(logs).includes(poison), false, 'no supplied strings');
    });
  }
}
