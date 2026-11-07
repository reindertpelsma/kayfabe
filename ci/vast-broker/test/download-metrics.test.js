import test from 'node:test';
import assert from 'node:assert/strict';
import { captureDownloadMetrics } from '../src/download-metrics.js';
import { setup } from './helpers.js';
// FALSIFIER: chunking loses/duplicates samples, hostile lines become measurements,
// or a claimed measurement changes pricing rather than remaining calibration data.
for (const split of [0, 25])
  test('download samples survive transport chunks ' + split, () => {
    const r = { offerFacts: { inet_down: 1562 } };
    const line = 'KF_DOWNLOAD_METRIC seconds=804.000000 bytes=42002015727\n';
    captureDownloadMetrics(r, line.slice(0, split));
    captureDownloadMetrics(r, line.slice(split));
    assert.equal(r.downloadMeasurements.advertised_inet_down, 1562);
    assert.equal(r.downloadMeasurements.samples[0].seconds, 804);
    assert.equal(r.downloadMeasurements.samples[0].bytes, 42002015727);
    assert.equal(r.downloadMeasurements.sum_transfer_seconds, 804);
    assert.equal(r.downloadMeasurements.source, 'payload_log_claim');
    assert.equal(r.downloadMeasurements.coverage, 'instrumented_curl_transfers_only');
  });
for (const line of [
  'seconds=NaN bytes=2',
  'seconds=-1 bytes=2',
  'seconds=1 bytes=https://secret.invalid',
  'seconds=999999 bytes=2',
  'seconds=1 bytes=9999999999999',
])
  test('invalid download sample is ignored: ' + line, () => {
    const r = {};
    captureDownloadMetrics(r, 'KF_DOWNLOAD_METRIC ' + line + '\n');
    assert.equal(r.downloadMeasurements, undefined);
  });
for (const count of [33, 80])
  test('download samples are bounded ' + count, () => {
    const r = {};
    captureDownloadMetrics(r, 'KF_DOWNLOAD_METRIC seconds=1 bytes=100\n'.repeat(count));
    assert.equal(r.downloadMeasurements.samples.length, 32);
    assert.equal(r.downloadMeasurements.sum_transfer_seconds, 32);
    assert.equal(r.downloadMeasurements.truncated, true);
  });
for (const split of [20, 30])
  test('worker records measurements once per accepted post ' + split, async () => {
    const h = await setup({ LOGS_BUCKET: { put: async () => ({}) } });
    assert.equal((await h.call()).status, 200);
    const line = 'KF_DOWNLOAD_METRIC seconds=804.000000 bytes=42002015727\n';
    const first = { seq: 1, kind: 'log', text: line.slice(0, split) };
    assert.equal((await h.report(first)).status, 200);
    assert.equal((await h.report(first)).status, 200);
    assert.equal((await h.report({ seq: 2, kind: 'log', text: line.slice(split) })).status, 200);
    const r = await h.storage.get('record:123-1');
    assert.equal(r.downloadMeasurements.samples.length, 1);
    assert.equal(r.downloadMeasurements.sum_transfer_seconds, 804);
    assert.equal(r.downloadMeasurements.advertised_inet_down, h.cloud.offer.inet_down);
    await h.call('/stop/123-1');
    const queued = await h.storage.get('plogq:123-1:0');
    assert.deepEqual(queued.downloadMeasurements, r.downloadMeasurements);
  });

for (const size of [500, 60000])
  test('download parser bounds and discards a long unfinished line ' + size, () => {
    const r = {};
    captureDownloadMetrics(r, 'x'.repeat(size) + 'KF_DOWNLOAD_METRIC seconds=1 bytes=2');
    assert.ok(r.downloadMetricTail.length <= 256);
    captureDownloadMetrics(r, '\n');
    assert.equal(r.downloadMeasurements, undefined);
    captureDownloadMetrics(r, 'KF_DOWNLOAD_METRIC seconds=2 bytes=10\n');
    assert.equal(r.downloadMeasurements.samples[0].seconds, 2);
  });

for (const secret of ['Bearer private-request-token', 'KF_DOWNLOAD_METRIC seconds=private-token'])
  test('arbitrary partial log text is not stored in metric state ' + secret.split(' ')[0], () => {
    const r = {};
    captureDownloadMetrics(r, secret);
    assert.equal(r.downloadMetricTail, '');
    assert.ok(!JSON.stringify(r).includes('private'));
  });
