import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration, gpuClass } from '../src/policy.js';
import { hardwareMatch } from '../src/release.js';
import { env } from './helpers.js';
// The Turing die rules (defaults.json dieRules), corrected 2026-11-06 after the review of #271:
// the GTX 1650 and 1630 are TU117 (the old rule GTX_16* said TU116) and the RTX 2070 is TU106 (the
// old rule RTX_2070* said TU104). The dies are public product knowledge, INFERRED, not measured on
// hardware. FALSIFIER: any of the 18 names below gets another die, or an ambiguous name gets one;
// the PASS hardware check disagrees with the offer rules; a Turing die leaves its family.

const cfg = configuration(env);
const offer = (gpu_name) => ({ gpu_arch: 'nvidia', compute_cap: 750, gpu_name });
const NAMES = [
  ['GTX 1630', 'TU117'],
  ['GTX 1650', 'TU117'],
  ['GTX 1650 SUPER', 'TU116'],
  ['GTX 1650 S', 'TU116'],
  ['GTX 1660', 'TU116'],
  ['GTX 1660 SUPER', 'TU116'],
  ['GTX 1660 S', 'TU116'],
  ['GTX 1660 Ti', 'TU116'],
  ['RTX 2060', 'TU106'],
  ['RTX 2060 SUPER', 'TU106'],
  ['RTX 2060 12GB', 'TU106'],
  ['RTX 2070', 'TU106'],
  ['RTX 2070 SUPER', 'TU104'],
  ['RTX 2070 S', 'TU104'],
  ['RTX 2080', 'TU104'],
  ['RTX 2080 SUPER', 'TU104'],
  ['RTX 2080 S', 'TU104'],
  ['RTX 2080 Ti', 'TU102'],
];

test('die rules: all 18 Turing names map to their die, with or without the vendor prefix', () => {
  assert.equal(NAMES.length, 18);
  for (const [name, die] of NAMES)
    for (const full of [name, 'NVIDIA GeForce ' + name, 'GeForce ' + name])
      assert.equal(gpuClass(offer(full), cfg), die, full);
});

test('die rules: unlisted names are not guessed', () => {
  // No rule names the 1650 Ti: it falls to its capability class, never to a die.
  assert.equal(gpuClass(offer('GTX 1650 Ti'), cfg), 'CC_75');
  // Names with a dash (Max-Q) and TITAN RTX are not admitted by the classifier at all.
  for (const name of ['GTX 1650 Max-Q', 'RTX 2060 Max-Q', 'RTX 2070 Max-Q SUPER', 'TITAN RTX'])
    assert.equal(gpuClass(offer(name), cfg), null, name);
});

test('die rules: no name matches two dies, and every Turing die stays in the Turing family', () => {
  const turing = cfg.dieRules.filter((r) => r.cc === 750);
  assert.deepEqual(turing.map((r) => r.die).sort(), ['TU102', 'TU104', 'TU106', 'TU116', 'TU117']);
  for (const [name] of NAMES) {
    const upper = name.toUpperCase().replaceAll(' ', '_');
    const hits = turing.filter((r) =>
      r.names.some((p) => new RegExp('^' + p.replaceAll('*', '.*') + '$').test(upper)),
    );
    assert.equal(hits.length, 1, name);
  }
  for (const die of ['TU102', 'TU104', 'TU106', 'TU116', 'TU117'])
    assert.equal(cfg.families.turing, 750, die);
});

test('die rules: the PASS hardware check uses the same rules (a 1650 box is no TU116)', () => {
  const box = (gpu_name) => ({
    hardware: { gpu_name, compute_cap: 750, driver_version: '580.159.04' },
  });
  const check = (name, die) => hardwareMatch(box(name), { die, family: 'turing', cfg });
  assert.equal(check('NVIDIA GeForce GTX 1650', 'TU117'), 'MATCH');
  assert.equal(
    check('NVIDIA GeForce GTX 1650', 'TU116'),
    'MISMATCH',
    'rented as TU116, a 1650 answered',
  );
  assert.equal(check('NVIDIA GeForce GTX 1660 Ti', 'TU116'), 'MATCH');
  assert.equal(check('NVIDIA GeForce RTX 2070', 'TU106'), 'MATCH');
  assert.equal(
    check('NVIDIA GeForce RTX 2070', 'TU104'),
    'MISMATCH',
    'rented as TU104, a 2070 answered',
  );
  assert.equal(check('NVIDIA GeForce RTX 2070 SUPER', 'TU104'), 'MATCH');
  assert.equal(check('NVIDIA GeForce RTX 2080 Ti', 'TU102'), 'MATCH');
});
