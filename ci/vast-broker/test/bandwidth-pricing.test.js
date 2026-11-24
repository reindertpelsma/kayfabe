import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULTS } from '../src/defaults.js';
import { resourceCaps } from '../src/policy.js';
import { estimateBandwidth, bandwidthEstimateInput, bandwidthOverrides } from '../src/bandwidth.js';
import { buildInstruction } from '../src/propimages.js';
// FALSIFIER: an estimate that is present and valid does not replace the profile number (it cannot lower it, or cannot raise it), one above the
// profile's hard maximum is not capped, an invalid one (not a number, zero, negative, NaN, infinite) changes anything, the owner's override stops
// raising, or a leg with no estimate prices differently from before.
const row = (tier) => DEFAULTS.payloadProfiles[tier];
const top = (tier) => DEFAULTS.payloadBandwidthMax[tier];
const profileOf = (tier) => (tier === 'large' ? 'release' : tier === 'adhoc' ? 'sweep' : 'check');
const caps = (tier, cfg = {}) => resourceCaps({ tierName: tier, ...cfg }, profileOf(tier), tier);

test('estimate: no estimate is the profile row, byte for byte (no new key)', () => {
  for (const tier of ['small', 'middle', 'large', 'adhoc']) assert.deepEqual(caps(tier), row(tier));
});
test('estimate: the hard maximum is data, never below the profile row, for every profile', () => {
  assert.deepEqual(
    Object.keys(DEFAULTS.payloadBandwidthMax).sort(),
    Object.keys(DEFAULTS.payloadProfiles).sort(),
  );
  for (const tier of Object.keys(DEFAULTS.payloadProfiles)) {
    assert.ok(top(tier).downGb >= row(tier).bandwidthDownGb, tier);
    assert.ok(top(tier).upGb >= row(tier).bandwidthUpGb, tier);
  }
});
for (const tier of ['small', 'middle', 'large', 'adhoc']) {
  test('estimate: a lower estimate LOWERS the profile number (' + tier + ')', () => {
    const c = caps(tier, { bandwidthEstimate: { downGb: 20.2, upGb: 0.4 } });
    assert.equal(c.bandwidthDownGb, 21);
    assert.equal(c.bandwidthUpGb, 1);
    assert.ok(c.bandwidthDownGb < row(tier).bandwidthDownGb);
    assert.deepEqual(c.estimate.source, ['down:estimate', 'up:estimate']);
    assert.deepEqual(c.estimate.capped, [false, false]);
    assert.equal(c.diskGb, row(tier).diskGb);
    assert.equal(c.maxInetUsdPerGb, row(tier).maxInetUsdPerGb);
  });
  test('estimate: a higher estimate RAISES it, up to the hard maximum (' + tier + ')', () => {
    const want = row(tier).bandwidthDownGb + 5;
    const c = caps(tier, { bandwidthEstimate: { downGb: want - 0.5, upGb: 3 } });
    assert.equal(c.bandwidthDownGb, want);
    assert.equal(c.bandwidthUpGb, 3);
  });
  test('estimate: above the hard maximum it is capped and says so (' + tier + ')', () => {
    const c = caps(tier, {
      bandwidthEstimate: { downGb: top(tier).downGb + 40, upGb: top(tier).upGb + 1 },
    });
    assert.equal(c.bandwidthDownGb, top(tier).downGb);
    assert.equal(c.bandwidthUpGb, top(tier).upGb);
    assert.deepEqual(c.estimate.capped, [true, true]);
  });
}
for (const bad of [0, -1, NaN, Infinity, -Infinity, '30', null, undefined, {}, [], true])
  test('estimate: an invalid number is ignored, the profile number stays: ' + String(bad), () => {
    const c = caps('large', { bandwidthEstimate: { downGb: bad, upGb: bad } });
    assert.equal(c.bandwidthDownGb, row('large').bandwidthDownGb);
    assert.equal(c.bandwidthUpGb, row('large').bandwidthUpGb);
    assert.deepEqual(c.estimate.source, ['down:profile', 'up:profile']);
  });
test('estimate: a zero download estimate never makes a leg free (the profile number stays)', () => {
  const c = caps('large', { bandwidthEstimate: { downGb: 0, upGb: 1 } });
  assert.equal(c.bandwidthDownGb, row('large').bandwidthDownGb);
  assert.equal(c.bandwidthUpGb, 1);
});
test('estimate: a zero upload estimate keeps the profile upload number', () => {
  const c = caps('large', { bandwidthEstimate: { downGb: 50, upGb: 0 } });
  assert.equal(c.bandwidthUpGb, row('large').bandwidthUpGb);
  assert.equal(c.bandwidthDownGb, 50);
});
test('estimate: down and up are judged separately', () => {
  const c = caps('large', { bandwidthEstimate: { downGb: 50, upGb: NaN } });
  assert.equal(c.bandwidthDownGb, 50);
  assert.equal(c.bandwidthUpGb, row('large').bandwidthUpGb);
  assert.deepEqual(c.estimate.source, ['down:estimate', 'up:profile']);
});
test('estimate: the owner override still only raises what the estimate gave', () => {
  const low = { bandwidthEstimate: { downGb: 30, upGb: 1 } };
  assert.equal(caps('large', { ...low, bandwidthDownOverride: 10 }).bandwidthDownGb, 30);
  assert.equal(caps('large', { ...low, bandwidthDownOverride: 200 }).bandwidthDownGb, 200);
  assert.equal(caps('large', { ...low, bandwidthUpOverride: 5 }).bandwidthUpGb, 5);
});
test('estimate: a missing or too-low hard maximum in the data is a configuration error, not a silent number', () => {
  const table = DEFAULTS.payloadBandwidthMax; // nested objects are not frozen: patched for one call, restored in finally
  const saved = table.large;
  try {
    for (const bad of [
      undefined,
      { downGb: 1, upGb: 10 },
      { downGb: 260, upGb: 0 },
      { downGb: 'x', upGb: 1 },
    ]) {
      table.large = bad;
      assert.throws(
        () => caps('large', { bandwidthEstimate: { downGb: 30, upGb: 1 } }),
        (e) => e.state === 'CONFIG_RESOURCE_PROFILE',
      );
    }
  } finally {
    table.large = saved;
  }
  assert.equal(caps('large', { bandwidthEstimate: { downGb: 30, upGb: 1 } }).bandwidthDownGb, 30);
});
test('estimate: a download maximum one below the profile row is refused, even for a small estimate under it', () => {
  // The hard maximum may never be below the profile's own row (defaults.json payloadBandwidthMax); a row the estimate would
  // not reach is still a configuration error, so the refusal does not depend on the estimate's size.
  const table = DEFAULTS.payloadBandwidthMax;
  const saved = table.large;
  try {
    table.large = { downGb: row('large').bandwidthDownGb - 1, upGb: row('large').bandwidthUpGb };
    assert.throws(
      () => caps('large', { bandwidthEstimate: { downGb: 1, upGb: 1 } }),
      (e) => e.state === 'CONFIG_RESOURCE_PROFILE',
    );
    table.large = { downGb: row('large').bandwidthDownGb, upGb: row('large').bandwidthUpGb };
    assert.equal(caps('large', { bandwidthEstimate: { downGb: 1, upGb: 1 } }).bandwidthDownGb, 1);
  } finally {
    table.large = saved;
  }
});
test('estimate: an estimate from an instruction prices the leg (the real input, not a hand-made one)', () => {
  const instruction = buildInstruction('release', {
    tierName: 'large',
    windowsRuns: 2,
    stages: ['specviewperf'],
    spec: 'lanea',
    rehearsal: true,
    propimages: true,
    die: 'AD102',
  });
  const e = estimateBandwidth(instruction, 'large');
  const input = bandwidthEstimateInput(e);
  assert.deepEqual(input, { downGb: e.down_gb, upGb: e.up_gb });
  const c = caps('large', { bandwidthEstimate: input });
  assert.equal(c.bandwidthDownGb, Math.min(Math.ceil(e.down_gb), top('large').downGb));
  assert.ok(c.bandwidthDownGb >= 90, 'two images on one die: the image bytes are in the number');
  // the same leg without an image list is far cheaper, so the estimate LOWERS the price of a plain leg
  const plain = estimateBandwidth(
    { ...instruction, propimages: undefined, propimages_strict: undefined },
    'large',
  );
  assert.ok(
    caps('large', { bandwidthEstimate: bandwidthEstimateInput(plain) }).bandwidthDownGb <
      row('large').bandwidthDownGb,
  );
});
test('estimate: the old override form still only raises (pinned)', () => {
  const profile = row('large').bandwidthDownGb;
  assert.equal(
    resourceCaps(bandwidthOverrides({ down_gb: 1, up_gb: 0.01 }), 'release').bandwidthDownGb,
    profile,
  );
  assert.equal(
    resourceCaps(bandwidthOverrides({ down_gb: profile + 50, up_gb: 3 }), 'release')
      .bandwidthDownGb,
    profile + 50,
  );
});
