// Similarity ranking of subruns (owner, 2026-11-07). A client names the subrun it wants (a die, an
// OS, a host and guest driver, a CPU type, a commit or tree); when no subrun matches exactly, the
// nearest ones are returned and every difference is explained. Pure functions over at most a few
// hundred candidate rows that the handler fetched with fixed statements: no I/O, no SQL here.
//
// Dimensions, in the default ladder order (the most acceptable thing to relax first):
//   1 commit        same tree > same day window (24 h) > older or newer by distance
//   2 cpu           same vendor and architecture > same architecture > other or unknown
//   3 host_driver   same > same branch (same major) > numeric distance
//   4 guest_driver  same > same branch > numeric distance
//   5 die           same die > nearest die in the same family (places in src/dies.js)
//   6 os            same > different
//   7 family        a die outside the family (this also relaxes `die`: a different family never
//                   has "the same die", so such a hit relaxes both, and is never "close")
// `relax` is the ordered list of dimensions a client lets the ranking relax (default: all seven in
// the order above); a dimension that is not in it must match exactly. Order of results (sort =
// closeness): fewer relaxed dimensions first, then the ladder position of the most-relaxed
// dimension (lower first), then the per-dimension closeness vector in ladder order (level, then
// distance), then recency (newer ended_at), then run id and subrun id (a total order, so two
// calls over the same rows return the same list and a cursor is stable).
import { familyOfDie, diePlaces } from './dies.js';

export const DIMS = Object.freeze([
  'commit',
  'cpu',
  'host_driver',
  'guest_driver',
  'die',
  'os',
  'family',
]);
export const DAY_MS = 24 * 3600 * 1000;
const FAR = 1e9;

// '580.159.03' -> [580, 159, 3]; null for anything else.
export function parseDriver(value) {
  const m = String(value ?? '').match(/^([0-9]{3,4})\.([0-9]{1,5})(?:\.([0-9]{1,5}))?$/);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3] ?? 0)] : null;
}
const sameTuple = (a, b) => a[0] === b[0] && a[1] === b[1] && a[2] === b[2];

function driverStep(wantedText, foundText) {
  const want = parseDriver(wantedText);
  const found = parseDriver(foundText);
  if (!want) return null; // nothing wanted
  if (!found) return { level: 3, dist: [FAR, FAR, FAR] };
  if (sameTuple(want, found)) return null;
  if (want[0] === found[0])
    return { level: 1, dist: [0, Math.abs(want[1] - found[1]), Math.abs(want[2] - found[2])] };
  return {
    level: 2,
    dist: [
      Math.abs(want[0] - found[0]),
      Math.abs(want[1] - found[1]),
      Math.abs(want[2] - found[2]),
    ],
  };
}

const cpuText = (vendor, arch) => (vendor || arch ? (vendor ?? '?') + '-' + (arch ?? '?') : null);

// The wanted CPU "amd" or "amd-x86_64" as {vendor, arch}; the architecture defaults from the vendor.
export function parseCpu(value) {
  const m = String(value ?? '').match(/^(amd|intel|arm|other)(?:-(x86_64|aarch64))?$/);
  if (!m) return null;
  return { vendor: m[1], arch: m[2] ?? (m[1] === 'arm' ? 'aarch64' : 'x86_64') };
}

// Compare ONE candidate to what was wanted. `wanted`: {commit, tree, die, os, host_driver,
// guest_driver, cpu:{vendor,arch}, refTime (ms or null)}; `active`: the dimensions in play (not
// ignored); `ctx.newest`: the newest candidate time, the reference when the wanted commit is unknown.
// Returns {exact:[dim], relaxed:[{dim, wanted, found, level, dist, note?}]}.
export function compare(c, wanted, active, ctx) {
  const exact = [];
  const relaxed = [];
  const ended = Date.parse(c.ended_at);
  const put = (dim, step, wantedText, foundText, note) => {
    if (step === null) exact.push(dim);
    else
      relaxed.push({
        dim,
        wanted: wantedText,
        found: foundText ?? null,
        level: step.level,
        dist: step.dist,
        ...(note ? { note } : {}),
      });
  };
  if (active.has('commit') && (wanted.tree || wanted.commit)) {
    const same =
      (wanted.tree && c.tree_hash === wanted.tree) ||
      (wanted.commit && c.commit_sha === wanted.commit);
    if (same) exact.push('commit');
    else {
      const ref = wanted.refTime ?? ctx.newest;
      const gap = Math.abs(ref - ended);
      const note =
        wanted.refTime === null || wanted.refTime === undefined
          ? 'reference_unknown'
          : ended < ref
            ? 'older'
            : 'newer';
      const within = wanted.refTime !== null && wanted.refTime !== undefined && gap <= DAY_MS;
      put(
        'commit',
        { level: within ? 1 : 2, dist: [within ? 0 : gap, 0, 0] },
        wanted.tree ?? wanted.commit,
        c.commit_sha,
        within ? 'same_day_' + note : note,
      );
    }
  }
  if (active.has('cpu') && wanted.cpu) {
    const cand = cpuText(c.cpu_vendor, c.cpu_arch);
    let step = null;
    if (!(c.cpu_vendor === wanted.cpu.vendor && c.cpu_arch === wanted.cpu.arch))
      step =
        c.cpu_arch !== null && c.cpu_arch === wanted.cpu.arch
          ? { level: 1, dist: [0, 0, 0] }
          : { level: 2, dist: [0, 0, 0] };
    put('cpu', step, cpuText(wanted.cpu.vendor, wanted.cpu.arch), cand);
  }
  if (active.has('host_driver') && wanted.host_driver)
    put(
      'host_driver',
      driverStep(wanted.host_driver, c.host_driver),
      wanted.host_driver,
      c.host_driver,
    );
  if (active.has('guest_driver') && wanted.guest_driver)
    put(
      'guest_driver',
      driverStep(wanted.guest_driver, c.guest_driver),
      wanted.guest_driver,
      c.guest_driver,
    );
  const wantedFamily = wanted.die ? familyOfDie(wanted.die) : null;
  const candFamily = c.family ?? familyOfDie(c.die);
  const crossFamily =
    wantedFamily !== null && wantedFamily !== 'unknown' && candFamily !== wantedFamily;
  if (active.has('die') && wanted.die) {
    if (c.die === wanted.die) exact.push('die');
    else if (crossFamily) put('die', { level: 2, dist: [FAR, 0, 0] }, wanted.die, c.die);
    else
      put('die', { level: 1, dist: [diePlaces(wanted.die, c.die) ?? 99, 0, 0] }, wanted.die, c.die);
  }
  if (active.has('os') && wanted.os)
    put('os', c.os === wanted.os ? null : { level: 1, dist: [0, 0, 0] }, wanted.os, c.os);
  if (active.has('family') && wantedFamily && wantedFamily !== 'unknown')
    put('family', crossFamily ? { level: 1, dist: [0, 0, 0] } : null, wantedFamily, candFamily);
  return { exact, relaxed };
}

// Total order over sort keys (arrays of numbers and strings).
export function compareKeys(a, b) {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] === b[i]) continue;
    if (typeof a[i] === 'number' && typeof b[i] === 'number') return a[i] - b[i];
    return String(a[i]) < String(b[i]) ? -1 : 1;
  }
  return 0;
}

// rank(candidates, wanted, {active, relax, sort, maxRelaxed}): the eligible candidates, sorted,
// each as {row, match, key}. `relax` is the effective ordered list of relaxable dimensions.
export function rank(candidates, wanted, { active, relax, sort = 'closeness', maxRelaxed = 7 }) {
  const position = new Map(relax.map((dim, i) => [dim, i + 1]));
  const times = candidates.map((c) => Date.parse(c.ended_at)).filter(Number.isFinite);
  const ctx = { newest: times.length ? Math.max(...times) : 0 };
  const out = [];
  for (const row of candidates) {
    const { exact, relaxed } = compare(row, wanted, active, ctx);
    if (relaxed.length > maxRelaxed || relaxed.some((r) => !position.has(r.dim))) continue;
    const ended = Date.parse(row.ended_at);
    const worst = relaxed.reduce((n, r) => Math.max(n, position.get(r.dim)), 0);
    const vector = relax.flatMap((dim) => {
      const r = relaxed.find((x) => x.dim === dim);
      return r ? [r.level, ...r.dist] : [0, 0, 0, 0];
    });
    const tail = [-(Number.isFinite(ended) ? ended : 0), row.run_id, row.sub];
    const closeness = [relaxed.length, worst, ...vector];
    const key =
      sort === 'recent'
        ? [...tail.slice(0, 1), ...closeness, ...tail.slice(1)]
        : sort === 'duration'
          ? [row.duration_ms ?? Infinity, ...closeness, ...tail]
          : [...closeness, ...tail];
    out.push({
      row,
      key,
      match: {
        exact,
        relaxed: relaxed
          .sort((x, y) => position.get(x.dim) - position.get(y.dim))
          .map(({ dim, wanted: w, found, note }) => ({
            dim,
            wanted: w,
            found,
            ...(note ? { note } : {}),
          })),
      },
    });
  }
  return out.sort((a, b) => compareKeys(a.key, b.key));
}
