// What a box runs and how long it may take: a pure function of the Worker profile.
// Owner ruling 2026-11-04 (lane 2): the master gate (profile `check`, gpu-vast.yml on a master PR or
// push) requires the Windows stage, one box, no SPECviewperf; the owner-dispatched die sweep stays
// Linux-only. The box reads KF_MERGE_WINDOWS (scripts/ci/vast_payload.py); a box from a Worker that
// predates the field runs the master gate, so the field can only narrow a sweep, never weaken a gate.
// tier.test.js pins the table.
export const TIERS = Object.freeze({
  check: Object.freeze({ windows: true }),
  sweep: Object.freeze({ windows: false }),
});

export function tier(profile) {
  if (!Object.hasOwn(TIERS, profile)) throw Error('unknown profile');
  return TIERS[profile];
}

// The box environment fields of the tier (string values, as Vast passes them).
export function tierEnvironment(profile) {
  return { KF_MERGE_WINDOWS: tier(profile).windows ? '1' : '0' };
}

// The hard runtime limit of a profile, in ms. cfg.hardCheck is HARD_RUNTIME_MINUTES (the master
// gate: Linux bar plus the Windows install and gate); cfg.hardSweep is SWEEP_HARD_RUNTIME_MINUTES,
// by default the smaller of 180 minutes and the check limit.
export function hardLimit(cfg, profile) {
  tier(profile);
  return profile === 'sweep' ? cfg.hardSweep : cfg.hardCheck;
}

// Apply a profile to a per-request configuration: every later price, reservation and deadline
// computation reads cfg.hard / cfg.ttl.
export function applyProfile(cfg, profile) {
  cfg.profile = profile;
  cfg.hard = hardLimit(cfg, profile);
  cfg.ttl = cfg.hard;
  return cfg;
}
