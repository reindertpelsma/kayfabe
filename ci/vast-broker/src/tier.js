// Tiers and profiles. A PROFILE is a budget and admission class (caps, price, hard limit, die set,
// host quality); a TIER is what a run is for (small, middle, large) and is chosen by the event that
// started it, never by a request field (V3_CI_TIERS_AND_IGNORE_LADDER.md §1, agreed 2026-11-05).
//
// Owner ruling 2026-11-04 (lane 2): the master gate (profile `check`, gpu-vast.yml on a master PR or
// push) requires the Windows stage, one box, no SPECviewperf; the owner-dispatched die sweep stays
// Linux-only. The box reads KF_MERGE_WINDOWS (scripts/ci/vast_payload.py); a box from a Worker that
// predates the field runs the master gate, so the field can only narrow a sweep, never weaken a gate.
// 2026-11-05 (wtier): profile `release` carries the large tier's legs; it is reachable only from the
// owner's admin start route, never from a GitHub token (src/workflow-authority.js has no release
// workflow). tier.test.js pins the tables.
import { releaseEnvironment } from './release-matrix.js';
import { DEFAULTS } from './defaults.js';
import { profileCaps, applyResourceProfile } from './policy.js';
export const TIERS = Object.freeze({
  check: Object.freeze({ windows: true }),
  sweep: Object.freeze({ windows: false }),
  release: Object.freeze({ windows: true }),
});
// What each tier means. `profile` is the budget class its rentals use. small and middle use the
// check profile (today's master-gate caps, Windows required, the check hard limit); large uses the
// release profile. `selection`: cheapest = today's check default (the cheapest of CHECK_DIES);
// rotate = the middle tier's family rotation with fallback; plan = the release planner
// (src/release.js). Until the small tier's own driver-patch matrix exists, small is exactly
// today's check run (so a master push is not weakened); that is a placeholder, not the design.
export const TIER_NAMES = Object.freeze(['small', 'middle', 'large']);
export const TIER_SETTINGS = Object.freeze({
  small: Object.freeze({ profile: 'check', selection: 'cheapest' }),
  middle: Object.freeze({ profile: 'check', selection: 'rotate' }),
  large: Object.freeze({ profile: 'release', selection: 'plan' }),
});
export function tier(profile) {
  if (!Object.hasOwn(TIERS, profile)) throw Error('unknown profile');
  return TIERS[profile];
}
export function tierSettings(name) {
  if (!Object.hasOwn(TIER_SETTINGS, name)) throw Error('unknown tier');
  return TIER_SETTINGS[name];
}
// The tier of an event (agreed recommendation 2026-11-05): a PR to master is middle, a release tag
// is large, a push or anything else is small. `sweep` (the owner-dispatched die sweep) is outside
// the three tiers and recorded as `adhoc`. Only the GitHub claims that admission has already
// verified reach this function; a release tag has no GitHub path into the Worker today (the OIDC
// identity check admits refs/heads/master only), so large is started from the admin route.
export function tierForEvent({ event, ref, baseRef, profile } = {}) {
  if (profile === 'sweep') return 'adhoc';
  if (event === 'pull_request_target' || event === 'pull_request')
    return baseRef === undefined || baseRef === 'master' ? 'middle' : 'small';
  if (event === 'push' && typeof ref === 'string' && /^refs\/tags\/v[0-9][^\s]*$/.test(ref))
    return 'large';
  return 'small';
}
// Review of PR #269, R4: the event, ref and base branch from the VERIFIED GitHub OIDC claims
// (signed by GitHub; admission has checked them). `base_ref` is the PR's base branch name; when a
// pull_request(_target) token lacks it, the event's `ref` (refs/heads/<base> for
// pull_request_target) names the base. A missing or malformed value leaves `baseRef` undefined.
export function tierForClaims(c, profile) {
  const branch = (v) =>
    typeof v === 'string' && /^[A-Za-z0-9._/-]{1,200}$/.test(v) ? v : undefined;
  const ref = typeof c?.ref === 'string' && c.ref.length <= 255 ? c.ref : undefined;
  const pr = c?.event_name === 'pull_request_target' || c?.event_name === 'pull_request';
  const fromRef = pr && ref?.startsWith('refs/heads/') ? branch(ref.slice(11)) : undefined;
  return tierForEvent({
    event: c?.event_name,
    ref,
    baseRef: branch(c?.base_ref) ?? fromRef,
    profile,
  });
}
// Tier contents, owner 2026-11-09. The large contract is intent, not an enabled executor.
export const TIER_CONTENTS = Object.freeze({
  small: Object.freeze({ base: 'check' }),
  middle: Object.freeze({
    inherits: 'small',
    appProfiles: Object.freeze({ linux: 'middle', windows: 'mid' }),
  }),
  large: Object.freeze({
    inherits: 'middle',
    additionalStages: Object.freeze(['specviewperf', 'llm_parity']),
    state: 'DESIGN_ONLY',
  }),
});
// Owner 2026-11-09: real apps are mandatory only for the signed middle tier.
// Callers request this exact pair; neither PR request fields nor informational box fields choose policy.
// Middle includes every supported Linux probe, including clpeak, gpu_burn and vkpeak.
export const MIDDLE_APP_COUNTS = Object.freeze({ apps: 66, windows_apps: 10 });
// A validated box PASS is still incomplete without the suites requested at admission.
// Only narrow PASS to an operator failure. Never relabel an attributed guest FAIL.
export function middleAppsResult(result, requested) {
  if (!requested || result.verdict !== 'PASS') return result;
  const missing = Object.entries(MIDDLE_APP_COUNTS).find(
    ([name, count]) => !result.suites.some((row) => row.name === name && row.pass === count),
  );
  if (!missing) return result;
  return {
    ...result,
    verdict: 'FAIL',
    suites: [
      { name: missing[0] + '_retry_operator', pass: 0, fail: 1, skip: 0 },
      ...result.suites.filter((row) => row.name !== missing[0] + '_retry_operator'),
    ].slice(0, 20),
  };
}
export function middleAppsRequest(profile, tierName, enabled) {
  return profile === 'check' && tierName === 'middle' && enabled
    ? { linuxApps: true, winApps: TIER_CONTENTS.middle.appProfiles.windows }
    : {};
}
// The family of a die class, from its compute capability in the die rules (one source of truth).
export function familyOf(die, cfg) {
  const rule = (cfg.dieRules ?? DEFAULTS.dieRules).find((r) => r.die === die);
  const cc = rule ? rule.cc : /^CC_([0-9]{2,3})$/.exec(die ?? '')?.[1] * 10;
  const families = cfg.families ?? DEFAULTS.families;
  return Object.keys(families).find((f) => families[f] === cc) ?? 'unknown';
}
// The explicit per-profile caps live in src/policy.js (the GitHub workflow bootstrap fetches only
// policy.js, github.js, workflow-authority.js and defaults from BASE, so policy.js imports nothing
// new) and are re-exported here.
export { profileCaps };
// The box environment fields of a run (string values, as Vast passes them). KF_TIER is
// informational on the box; KF_WINDOWS_RUNS (2 = the release tier's second Windows run on the same
// box when no second machine of the die is offered) and KF_STAGES (extra stages, release only) are
// sent only when they differ from the default, so a check or sweep box sees today's environment
// plus KF_TIER.
// Owner 2026-11-09: the trusted BASE runner requests Linux apps and winApps=mid for the signed
// middle tier when MIDDLE_APPS_ENABLED is not false. Admission independently enforces the pair.
// Small/push and adhoc/sweep never request them. Source changes require a matching bundle pin.
// 2026-11-07 (SPECviewperf stage, PROPOSED, docs/design/V3_SPEC_GATE.md): `spec` (lanea, laneb, both) is sent as KF_SPEC
// only when the caller passes it, only for the release profile and only together with the specviewperf stage slot (the box
// refuses either alone). The release planner assigns lanes only behind the disabled stage flag;
// the proposed matrix and absent executor independently prevent rentals.
export function tierEnvironment(
  profile,
  { tierName, windowsRuns = 1, stages = [], releaseMatrix, spec, winApps, linuxApps } = {},
) {
  const env = { KF_MERGE_WINDOWS: tier(profile).windows ? '1' : '0' };
  if (tierName !== undefined) env.KF_TIER = tierName;
  if (windowsRuns !== 1) {
    if (profile !== 'release' || windowsRuns !== 2) throw Error('windows runs');
    env.KF_WINDOWS_RUNS = '2';
  }
  if (stages.length) {
    if (profile !== 'release' || stages.some((s) => !/^[a-z][a-z0-9_]{0,31}$/.test(s)))
      throw Error('stages');
    env.KF_STAGES = stages.join(':');
  }
  if (releaseMatrix !== undefined) {
    if (profile !== 'release') throw Error('release matrix profile');
    Object.assign(env, releaseEnvironment(releaseMatrix));
  }
  if (linuxApps !== undefined) {
    if (profile !== 'check' || tierName !== 'middle' || linuxApps !== true)
      throw Error('linuxapps');
    env.KF_LINUX_APPS = '1';
  }
  if (winApps !== undefined) {
    // needs the Windows stage, and only a named profile
    if (!tier(profile).windows || !['small', 'mid', 'full'].includes(winApps))
      throw Error('winapps');
    env.KF_WINAPPS = winApps;
  }
  if (spec !== undefined) {
    if (
      profile !== 'release' ||
      !stages.includes('specviewperf') ||
      !['lanea', 'laneb', 'both'].includes(spec)
    )
      throw Error('spec');
    env.KF_SPEC = spec;
  }
  return env;
}
// The hard runtime limit of a profile, in ms. cfg.hardCheck is HARD_RUNTIME_MINUTES (the master
// gate: Linux bar plus the Windows install and gate); cfg.hardSweep is SWEEP_HARD_RUNTIME_MINUTES,
// by default the smaller of 180 minutes and the check limit; cfg.hardRelease is
// RELEASE_HARD_RUNTIME_MINUTES (default 300, the check rule: one leg runs the Linux bar plus the
// Windows install and up to two Windows gate runs).
export function hardLimit(cfg, profile) {
  return profileCaps(cfg, profile).hard;
}
// Apply a profile to a per-request configuration: every later price, reservation and deadline
// computation reads cfg.hard / cfg.ttl.
export function applyProfile(cfg, profile, tierName) {
  applyResourceProfile(cfg, profile, tierName);
  cfg.profile = profile;
  cfg.hard = hardLimit(cfg, profile);
  cfg.ttl = cfg.hard;
  return cfg;
}
