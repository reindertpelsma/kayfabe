// ci/requirements.toml: which results a tree key needs before CI counts a tier as met.
// Read by the CI Worker from the BASE branch (never from the PR), so a PR cannot shrink what it is
// tested against. Design: docs/design/V3_PLATFORM_TESTING.md §1/§4 and
// docs/design/V3_CI_TIERS_AND_IGNORE_LADDER.md §1. Format reference: ci/vast-broker/README.md.
//
// The file is a strict TOML subset (every valid file is valid TOML; test/requirements.test.js
// parses ci/requirements.toml with Python's tomllib too):
//   version = 1
//   [[require]]
//   tier = "middle"                       # small | middle | large
//   family = "any"                        # a family id (turing, ampere, ada, hopper, blackwell) or "any"
//   dies = ["any"]                        # die ids (GA102, CC_89, ...) or ["any"]
//   min_dies = 1                          # with dies = ["any"]: distinct dies that must pass
//   platforms = ["linux", "windows"]      # each must be covered; a 'both' run covers both
//   stages = ["gates", "windows_gate"]    # each must be PASS in the answering run
//   min_passes = 1                        # verified PASS runs needed per die and platform
// Comments start with '#'. Strings use "..." without escapes. Anything else is refused, and a
// refused or missing file means "not met" (fail closed).

export const TIERS = Object.freeze(['small', 'middle', 'largest']);
export const TIER_RANK = Object.freeze({ adhoc: 0, small: 1, middle: 2, largest: 3 });
export const PLATFORMS = Object.freeze(['linux', 'windows']);
export const MAX_BYTES = 65536;
export const MAX_BLOCKS = 64;
// An in-flight row older than this is ignored (the check profile's hard limit is 300 minutes).
export const INFLIGHT_MAX_MS = 8 * 3600 * 1000;

export class RequirementsError extends Error {
  constructor(line, why) {
    super('requirements line ' + line + ': ' + why);
    this.line = line;
    this.why = why;
  }
}

// 'large' is the tier's name in the lane briefs; the schema and the public API spell it 'largest'.
export function normalTier(value) {
  if (value === 'large') return 'largest';
  if (typeof value === 'string' && Object.hasOwn(TIER_RANK, value)) return value;
  return null;
}

export const DIE = /^(?:[A-Z]{2}[0-9]{3}[A-Z]?|CC_[0-9]{2,3})$/;
export const FAMILY = /^[a-z][a-z0-9]{2,15}$/;
export const STAGE = /^[a-z][a-z0-9_]{0,47}$/;
const PREFIX_FAMILY = { TU: 'turing', GA: 'ampere', AD: 'ada', GH: 'hopper', GB: 'blackwell' };
const CC_FAMILY = {
  75: 'turing',
  80: 'ampere',
  86: 'ampere',
  87: 'ampere',
  89: 'ada',
  90: 'hopper',
  100: 'blackwell',
  120: 'blackwell',
};

// The family of a die id or compute-capability class; 'unknown' when it has none.
export function familyOf(die) {
  if (typeof die !== 'string' || !DIE.test(die)) return 'unknown';
  if (die.startsWith('CC_')) return CC_FAMILY[die.slice(3)] ?? 'unknown';
  return PREFIX_FAMILY[die.slice(0, 2)] ?? 'unknown';
}

const STRING = /^"([A-Za-z0-9_.+-]*)"$/;
function value(raw, line) {
  let m;
  if ((m = STRING.exec(raw))) return m[1];
  if (/^(?:0|[1-9][0-9]{0,3})$/.test(raw)) return Number(raw);
  if ((m = /^\[(.*)\]$/.exec(raw))) {
    const inner = m[1].trim();
    if (inner === '') return [];
    const items = inner.replace(/,\s*$/, '').split(',');
    return items.map((item) => {
      const s = STRING.exec(item.trim());
      if (!s) throw new RequirementsError(line, 'array items must be strings');
      return s[1];
    });
  }
  throw new RequirementsError(line, 'unsupported value');
}

function stripComment(text) {
  let inString = false;
  for (let i = 0; i < text.length; i++) {
    if (text[i] === '"') inString = !inString;
    else if (text[i] === '#' && !inString) return text.slice(0, i);
  }
  return text;
}

const KEYS = new Set(['tier', 'family', 'dies', 'min_dies', 'platforms', 'stages', 'min_passes']);

function checkBlock(raw, line) {
  const fail = (why) => {
    throw new RequirementsError(line, why);
  };
  for (const k of ['tier', 'family', 'dies', 'platforms'])
    if (!Object.hasOwn(raw, k)) fail('missing ' + k);
  const tier = raw.tier === 'large' ? 'largest' : raw.tier;
  if (!TIERS.includes(tier)) fail('tier');
  if (raw.family !== 'any' && (typeof raw.family !== 'string' || !FAMILY.test(raw.family)))
    fail('family');
  const list = (v, test, max) =>
    Array.isArray(v) &&
    v.length >= 1 &&
    v.length <= max &&
    new Set(v).size === v.length &&
    v.every((x) => typeof x === 'string' && test(x));
  const anyDie = Array.isArray(raw.dies) && raw.dies.length === 1 && raw.dies[0] === 'any';
  if (!anyDie && !list(raw.dies, (d) => DIE.test(d), 32)) fail('dies');
  if (!anyDie && raw.family !== 'any' && raw.dies.some((d) => familyOf(d) !== raw.family))
    fail('a die outside the family');
  if (!list(raw.platforms, (p) => PLATFORMS.includes(p), 2)) fail('platforms');
  const stages = raw.stages ?? [];
  if (!(Array.isArray(stages) && stages.length === 0) && !list(stages, (s) => STAGE.test(s), 32))
    fail('stages');
  const count = (v, fallback, max) => {
    if (v === undefined) return fallback;
    if (!Number.isSafeInteger(v) || v < 1 || v > max) fail('count');
    return v;
  };
  if (raw.min_dies !== undefined && !anyDie) fail('min_dies needs dies = ["any"]');
  return {
    tier,
    family: raw.family,
    dies: anyDie ? 'any' : [...raw.dies],
    min_dies: count(raw.min_dies, 1, 32),
    platforms: [...raw.platforms],
    stages: [...stages],
    min_passes: count(raw.min_passes, 1, 8),
  };
}

// Text -> {version, blocks}. Throws RequirementsError (fail closed) on anything outside the subset.
export function parseRequirements(text) {
  if (typeof text !== 'string') throw new RequirementsError(0, 'no text');
  if (new TextEncoder().encode(text).length > MAX_BYTES) throw new RequirementsError(0, 'size');
  if (text.includes('\r')) throw new RequirementsError(0, 'CR');
  let version = null,
    current = null,
    currentLine = 0;
  const blocks = [];
  const finish = () => {
    if (current) blocks.push(checkBlock(current, currentLine));
  };
  text.split('\n').forEach((rawLine, index) => {
    const line = index + 1;
    const body = stripComment(rawLine).trim();
    if (body === '') return;
    if (body === '[[require]]') {
      finish();
      current = {};
      currentLine = line;
      if (blocks.length >= MAX_BLOCKS) throw new RequirementsError(line, 'too many blocks');
      return;
    }
    const m = /^([a-z_]{1,32})\s*=\s*(.+)$/.exec(body);
    if (!m) throw new RequirementsError(line, 'not key = value');
    const [, key, raw] = m;
    const parsed = value(raw.trim(), line);
    if (current === null) {
      if (key !== 'version' || version !== null || parsed !== 1)
        throw new RequirementsError(line, 'only version = 1 before the first [[require]]');
      version = parsed;
      return;
    }
    if (!KEYS.has(key)) throw new RequirementsError(line, 'unknown key ' + key);
    if (Object.hasOwn(current, key)) throw new RequirementsError(line, 'duplicate key ' + key);
    current[key] = parsed;
  });
  finish();
  if (version !== 1) throw new RequirementsError(0, 'version = 1 is required');
  return { version, blocks };
}

const covers = (row, platform) => row.platform === platform || row.platform === 'both';
const newestFirst = (a, b) =>
  a.sort_at < b.sort_at ? 1 : a.sort_at > b.sort_at ? -1 : a.run_id < b.run_id ? 1 : -1;

// rows: ci_runs rows of ONE result key, each with `valid` (its HMAC tag verified) and `stages`
// ([{name, result}]). Returns {met, missing, inflight} for the blocks of `tier`.
export function evaluate(blocks, tier, rows, now) {
  const wanted = blocks.filter((b) => b.tier === tier);
  if (!wanted.length)
    return {
      met: false,
      missing: [{ family: 'any', die: 'any', platform: 'any', reason: 'NO_REQUIREMENTS' }],
      inflight: [],
    };
  const missing = [],
    inflight = new Set();
  for (const block of wanted) {
    const eligible = rows.filter(
      (r) =>
        r.source === 'reproduced' &&
        TIER_RANK[r.tier] >= TIER_RANK[block.tier] &&
        (block.family === 'any' || r.family === block.family),
    );
    const running = eligible.filter(
      (r) => r.verdict === null && Date.parse(r.started_at) > now - INFLIGHT_MAX_MS,
    );
    const finals = eligible
      .filter((r) => r.valid === true && (r.verdict === 'PASS' || r.verdict === 'FAIL'))
      .sort(newestFirst);
    const stagesPass = (r) =>
      block.stages.every((name) => r.stages.some((s) => s.name === name && s.result === 'PASS'));
    // The answer of one die on one platform: the newest final PASS/FAIL row answers.
    const cell = (die, platform) => {
      const answers = finals.filter((r) => r.die === die && covers(r, platform));
      if (answers.length) {
        if (answers[0].verdict === 'FAIL') return 'FAIL';
        if (!stagesPass(answers[0])) return 'STAGE';
        const passes = answers.filter((r) => r.verdict === 'PASS' && stagesPass(r)).length;
        return passes >= block.min_passes ? 'OK' : 'TOO_FEW_PASSES';
      }
      return running.some((r) => r.die === die && covers(r, platform)) ? 'INFLIGHT' : 'NO_RESULT';
    };
    const join = (die) => {
      for (const r of running)
        if ((die === null || r.die === die) && block.platforms.some((p) => covers(r, p)))
          inflight.add(r.run_id);
    };
    if (block.dies !== 'any') {
      for (const die of block.dies)
        for (const platform of block.platforms) {
          const state = cell(die, platform);
          if (state === 'OK') continue;
          missing.push({ family: familyOf(die), die, platform, reason: state });
          if (state === 'INFLIGHT') join(die);
        }
      continue;
    }
    const dies = [...new Set(eligible.map((r) => r.die))].sort();
    const states = dies.map((die) => block.platforms.map((p) => cell(die, p)));
    const ok = states.filter((s) => s.every((x) => x === 'OK')).length;
    if (ok >= block.min_dies) continue;
    const all = states.flat();
    const reason = all.includes('FAIL')
      ? 'FAIL'
      : all.includes('INFLIGHT')
        ? 'INFLIGHT'
        : all.includes('STAGE')
          ? 'STAGE'
          : all.includes('TOO_FEW_PASSES')
            ? 'TOO_FEW_PASSES'
            : 'NO_RESULT';
    missing.push({
      family: block.family,
      die: 'any',
      platform: block.platforms.join('+'),
      reason,
      need: block.min_dies,
      have: ok,
    });
    join(null);
  }
  return { met: missing.length === 0, missing, inflight: [...inflight].sort() };
}
