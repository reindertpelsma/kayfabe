import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { setup, sha, reporterSource } from './helpers.js';
import { DEFAULTS } from '../src/defaults.js';
// FALSIFIER V6: a tested commit whose reporter cannot speak the Worker's box protocol (any
// commit before this round) still rents and bills until BOOT_TIMEOUT.
const marker = 'BOX_PROTOCOL = "' + DEFAULTS.boxProtocol + '"';
const vastCalls = (h) => h.cloud.calls.filter((x) => x.url.startsWith('https://console.vast.ai'));
test('round repository reporter declares the Worker box protocol', async () => {
  const text = await readFile(new URL('../../../scripts/ci/vast_report.py', import.meta.url));
  assert.match(String(text), new RegExp('^' + marker + '$', 'm'));
  assert.match(String(text), /^PROTOCOLS = \{BOX_PROTOCOL\}$/m);
});
for (const [name, patch] of [
  ['missing', (h) => (h.cloud.reporterText = reporterSource.replace(/^BOX_PROTOCOL = .*$/m, ''))],
  ['older', (h) => (h.cloud.reporterText = 'BOX_PROTOCOL = "5"\nPROTOCOLS = {"4", "5"}\n')],
  ['newer', (h) => (h.cloud.reporterText = 'BOX_PROTOCOL = "99"\n')],
  ['indented', (h) => (h.cloud.reporterText = '    ' + marker + '\n')],
  ['absent-file', (h) => (h.cloud.reporterStatus = 404)],
  ['not-base64', (h) => (h.cloud.reporterEncoding = 'none')],
])
  test('round box protocol marker refuses before any Vast call ' + name, async () => {
    const h = await setup();
    patch(h);
    const response = await h.call();
    assert.notEqual(response.status, 200);
    assert.equal(h.cloud.creates, 0);
    assert.deepEqual(vastCalls(h), []);
  });
for (const restart of [false, true])
  test('round box protocol is read at the exact tested sha ' + restart, async () => {
    const h = await setup();
    if (restart) h.restart();
    assert.equal((await h.call()).status, 200);
    const urls = h.cloud.calls.map((x) => x.url);
    const read = urls.findIndex((x) =>
      x.endsWith('/contents/scripts/ci/vast_report.py?ref=' + sha),
    );
    assert.ok(read >= 0);
    assert.ok(read < urls.findIndex((x) => x.startsWith('https://console.vast.ai')));
  });
for (const name of ['KF_PROTOCOL', 'marker'])
  test('round Worker box protocol matches the marker ' + name, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    const env = h.cloud.calls.find((x) => x.url.includes('/asks/')).body.env;
    if (name === 'KF_PROTOCOL') assert.equal(String(env.KF_PROTOCOL), String(DEFAULTS.boxProtocol));
    else assert.ok(reporterSource.includes('BOX_PROTOCOL = "' + env.KF_PROTOCOL + '"'));
  });
