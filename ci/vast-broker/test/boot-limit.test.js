import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { DEFAULTS } from '../src/defaults.js';
import { configuration } from '../src/policy.js';
import { env } from './helpers.js';
// Owner 2026-10-26: KVM startup on Vast takes up to ~30 minutes on some hosts, and the box then
// clones, checks out and downloads the setup cache before its first report. A 40 minute boot limit
// destroyed healthy slow boxes.
// FALSIFIER: the default boot limit is below 60 minutes, reaches the hard limit, or the stored
// deadlines disagree with it.
test('default boot limit leaves slow KVM hosts room and stays below the hard limit', () => {
  assert.equal(DEFAULTS.bootMinutes, 75);
  assert.ok(DEFAULTS.bootMinutes >= 60);
  assert.ok(DEFAULTS.bootMinutes < DEFAULTS.hardMinutes);
  assert.equal(DEFAULTS.hardMinutes, 180);
  // A box that boots at the limit must still have the soft margin and a heartbeat before the hard stop.
  assert.ok(
    DEFAULTS.bootMinutes + DEFAULTS.heartbeatTimeoutMinutes + DEFAULTS.softMarginMinutes <
      DEFAULTS.hardMinutes,
  );
});
test('configuration() uses the default boot limit unless the owner overrides it', () => {
  const bare = { ...env, BOOT_LIMIT_MINUTES: undefined, HARD_RUNTIME_MINUTES: undefined };
  const c = configuration(bare);
  assert.equal(c.boot, 75 * 60000);
  assert.ok(c.boot < c.hard - c.softMargin);
  assert.equal(configuration({ ...bare, BOOT_LIMIT_MINUTES: '30' }).boot, 30 * 60000);
});
test('the owner-facing docs state the same default', () => {
  const deploy = readFileSync(new URL('../DEPLOY.md', import.meta.url), 'utf8');
  assert.match(deploy, /75-minute boot/);
  assert.doesNotMatch(deploy, /40-minute boot/);
});
