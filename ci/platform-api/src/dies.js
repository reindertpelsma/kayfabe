// Die order inside a family, smallest chip first, for the subrun similarity ranking (src/rank.js).
// The dies are the repository's own lists: ci/vast-broker/defaults.json `dieRules` (with the
// product-data corrections of 2026-11-06: GTX 1650/1630 are TU117, RTX 2070 is TU106, only the
// RTX 2070 SUPER is TU104) and `sweepDies`, plus the two chips the repository names as not yet
// covered (GA107, GA103; docs/design/V3_PLATFORM_TESTING.md and the 2026-11-05 batch notes) and the
// Hopper die. test/rank.test.js cross-checks this table against defaults.json, so a die added there
// must be placed here. Order is by chip size within one architecture (the number in the name:
// larger is bigger for TU10x, GA10x, AD10x; GB20x runs 207 < 206 < 205 < 203 < 202 by the same
// convention). It is an ORDER, not a distance in any unit: proximity is the number of places
// between two dies.
export const DIE_ORDER = Object.freeze({
  turing: Object.freeze(['TU117', 'TU116', 'TU106', 'TU104', 'TU102']),
  ampere: Object.freeze(['GA107', 'GA106', 'GA104', 'GA103', 'GA102']),
  ada: Object.freeze(['AD107', 'AD106', 'AD104', 'AD103', 'AD102']),
  hopper: Object.freeze(['GH100']),
  blackwell: Object.freeze(['GB207', 'GB206', 'GB205', 'GB203', 'GB202']),
});

const PREFIX = Object.freeze({
  TU: 'turing',
  GA: 'ampere',
  AD: 'ada',
  GH: 'hopper',
  GB: 'blackwell',
});
// Compute-capability classes ("CC_89") stand for a family without naming a die.
const CC_FAMILY = Object.freeze({
  75: 'turing',
  80: 'ampere',
  86: 'ampere',
  87: 'ampere',
  89: 'ada',
  90: 'hopper',
  100: 'blackwell',
  120: 'blackwell',
});

export function familyOfDie(die) {
  if (typeof die !== 'string') return 'unknown';
  if (/^CC_[0-9]{2,3}$/.test(die)) return CC_FAMILY[die.slice(3)] ?? 'unknown';
  if (!/^[A-Z]{2}[0-9]{3}[A-Z]?$/.test(die)) return 'unknown';
  return PREFIX[die.slice(0, 2)] ?? 'unknown';
}

// Places between two dies of ONE family; null when either is not in the order table (a CC class,
// or a die the table does not know), so an unknown die is never ranked as "close".
export function diePlaces(a, b) {
  const family = familyOfDie(a);
  if (family === 'unknown' || family !== familyOfDie(b)) return null;
  const order = DIE_ORDER[family];
  const i = order.indexOf(a);
  const j = order.indexOf(b);
  return i < 0 || j < 0 ? null : Math.abs(i - j);
}
