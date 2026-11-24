// The app lane planner: when a release requires the app lane, which boxes would run which image on which family, and what it costs.
// STATUS: DESIGN-ONLY for the Worker, 2026-11-18. This module is pure, tested and GPU-free; nothing in src/index.js imports it
// (the Worker admin start kind 'app' is NOT built), so it is not part of the deployed bundle and rents nothing.
// Ruling: docs/OWNER_RULINGS.md section AZ. Design: docs/design/V3_APP_LANE_PLANNER.md. Data: ci/vast-broker/app-lane.json.
//
// The planner never names an image: images, their needs and the core paths are configuration (app-lane.json), the
// pinned sizes come from propimages.json and the downloads per stage from bandwidth.json. Every configured image is a bundle of
// propimage-bundles.json (src/bundles.js), and an application list is turned into the images to run by `bundlesFor` (option `apps`).
import appLane from '../app-lane.json' with { type: 'json' };
import { BANDWIDTH, estimateBandwidth, plannedStages } from './bandwidth.js';
import { BUNDLES, bundlesFor } from './bundles.js';
import { DEFAULTS } from './defaults.js';
import { RELEASE_MATRIX } from './release-matrix.js';
import { familyOf } from './tier.js';

export const APP_LANE = appLane;
const GB = 1e9;
const round = (x) => Math.round(x * 10000) / 10000;
const isObject = (v) => v !== null && typeof v === 'object' && !Array.isArray(v);
const posInt = (n) => Number.isSafeInteger(n) && n > 0;

// ---- core paths ----------------------------------------------------------------------------------------------------------
const globProblem = (glob) => {
  if (typeof glob !== 'string') return 'not a string';
  const body = glob.startsWith('!') ? glob.slice(1) : glob;
  if (!body) return 'empty';
  if (/[[\]{}?\\\0]/.test(body)) return 'unsupported character (only * and ** are globs)';
  if (body.startsWith('/') || body.endsWith('/')) return 'leading or trailing slash';
  if (body.split('/').some((s) => s === '' || s === '.' || s === '..'))
    return 'empty or dot segment';
  if (/\*\*[^/]|[^/]\*\*/.test(body)) return '** must be a whole segment';
  return null;
};
// '*' stays inside one path segment; '**' crosses segments ('a/**' needs something below a, 'a/**/c' also matches a/c).
export function globToRegExp(glob) {
  const problem = globProblem(glob);
  if (problem) throw Error('glob ' + JSON.stringify(glob) + ': ' + problem);
  let out = '';
  for (let i = 0; i < glob.length; ) {
    if (glob.startsWith('**/', i)) {
      out += '(?:.*/)?';
      i += 3;
    } else if (glob.startsWith('**', i)) {
      out += '.+';
      i += 2;
    } else if (glob[i] === '*') {
      out += '[^/]*';
      i += 1;
    } else {
      out += glob[i].replace(/[.+^${}()|[\]\\?]/g, '\\$&');
      i += 1;
    }
  }
  return new RegExp('^' + out + '$');
}
function normalPath(file) {
  if (typeof file !== 'string' || !file) throw Error('changed file: not a non-empty string');
  let p = file;
  while (p.startsWith('./')) p = p.slice(2);
  if (p.includes('\\') || p.includes('\0') || p.startsWith('/'))
    throw Error('changed file: ' + JSON.stringify(file));
  if (p.split('/').some((s) => s === '' || s === '..' || s === '.'))
    throw Error('changed file: ' + JSON.stringify(file));
  return p;
}
// Does a change need the app lane? `corePaths`: globs, a leading '!' excludes. A file is core when a pattern without '!' matches
// it and none with '!' does. Returns { required, matches, excluded }: the core files, and the files that matched a core pattern
// but are excepted. Pass renames as both names (git diff --name-status or --no-renames).
export function appLaneRequired(changedFiles, corePaths) {
  if (!Array.isArray(changedFiles)) throw Error('changedFiles must be an array');
  if (
    !Array.isArray(corePaths) ||
    !corePaths.some((g) => typeof g === 'string' && !g.startsWith('!'))
  )
    throw Error('core_paths: needs at least one pattern');
  const include = corePaths.filter((g) => !g.startsWith('!')).map(globToRegExp);
  const exclude = corePaths.filter((g) => g.startsWith('!')).map((g) => globToRegExp(g.slice(1)));
  const matches = [],
    excluded = [];
  for (const file of new Set(changedFiles.map(normalPath))) {
    if (!include.some((re) => re.test(file))) continue;
    (exclude.some((re) => re.test(file)) ? excluded : matches).push(file);
  }
  return { required: matches.length > 0, matches, excluded };
}

// ---- configuration -------------------------------------------------------------------------------------------------------
export function validateAppLaneConfig(config, table = BANDWIDTH, bundles = BUNDLES) {
  const errors = [];
  if (!isObject(config)) return ['config: not an object'];
  if (!Array.isArray(config.core_paths)) errors.push('core_paths: not an array');
  else {
    for (const g of config.core_paths) {
      const problem = globProblem(g);
      if (problem) errors.push('core_paths ' + JSON.stringify(g) + ': ' + problem);
    }
    if (!config.core_paths.some((g) => typeof g === 'string' && g && !g.startsWith('!')))
      errors.push('core_paths: no pattern');
  }
  if (!posInt(config.rotation?.window_releases)) errors.push('rotation.window_releases');
  const role = config.roles?.image;
  if (!posInt(role?.hard_minutes)) errors.push('roles.image.hard_minutes');
  if (!(role?.max_inet_usd_per_gb > 0 && role.max_inet_usd_per_gb <= 1))
    errors.push('roles.image.max_inet_usd_per_gb');
  if (!Array.isArray(config.images) || !config.images.length) errors.push('images: none');
  const seen = new Set();
  for (const img of Array.isArray(config.images) ? config.images : []) {
    const id = img?.id;
    if (typeof id !== 'string' || !/^[a-z0-9_]{1,40}$/.test(id)) errors.push('image id');
    else if (seen.has(id)) errors.push('image duplicate: ' + id);
    else if (!Object.hasOwn(bundles?.bundles ?? {}, id))
      errors.push('image ' + id + ': not a bundle of propimage-bundles.json');
    seen.add(id);
    if (!posInt(img?.disk_gb)) errors.push('image ' + id + ': disk_gb');
    if (!(typeof img?.vram_gb === 'number' && Number.isFinite(img.vram_gb) && img.vram_gb > 0))
      errors.push('image ' + id + ': vram_gb');
    if (typeof img?.manual_only !== 'boolean') errors.push('image ' + id + ': manual_only');
    if (!Array.isArray(img?.stages) || !img.stages.length) errors.push('image ' + id + ': stages');
    for (const s of img?.stages ?? [])
      if (!isObject(table.stages[s]))
        errors.push('image ' + id + ': stage ' + s + ' is not in bandwidth.json');
  }
  return errors;
}
const configProblems = validateAppLaneConfig(APP_LANE);
if (configProblems.length) throw Error('app-lane.json: ' + configProblems.join('; '));

// ---- the profile of one image role ---------------------------------------------------------------------------------------
// Bandwidth is the measured GB of the image's own stages (bandwidth.json, with its margin), not the release profile's flat figure.
// A stage that bandwidth.js does not plan on its own (a table stage with no entries of its own) adds its upload only.
export function imageProfile(image, { table = BANDWIDTH } = {}) {
  const instruction = { tier: 'large', windows: false, propimages: [{ label: image.id }] };
  const planned = plannedStages(instruction, { only: image.stages });
  const only = image.stages.filter((s) => planned.includes(s));
  const extra = image.stages.filter((s) => !planned.includes(s));
  const estimate = estimateBandwidth(instruction, 'large', { table, only });
  let up_bytes = estimate.up_bytes;
  for (const s of extra) {
    if (table.stages[s].entries.length) throw Error('bandwidth stage not plannable: ' + s);
    up_bytes += Math.ceil((table.stages[s].upload.up_bytes * (100 + table.margin.percent)) / 100);
  }
  return {
    image: image.id,
    disk_gb: image.disk_gb,
    vram_gb: image.vram_gb,
    bandwidth_down_gb: estimate.down_gb,
    bandwidth_up_gb: Math.ceil((up_bytes * 100) / GB) / 100,
    stages: [...image.stages],
    manual_only: image.manual_only,
  };
}
// The worst-case price of one leg, by the same formula as policy.js offerPrice but with the image's own disk and bandwidth
// (test/app-lane.test.js: equal to offerPrice().total for the release profile numbers). `hard`, `cronPeriod` in ms.
export function appLegTotal(offer, profile, { hard, cronPeriod }) {
  for (const key of ['dph_base', 'storage_cost', 'inet_down_cost', 'inet_up_cost'])
    if (typeof offer[key] !== 'number' || !Number.isFinite(offer[key]) || offer[key] < 0)
      return null;
  const storage = (offer.storage_cost * profile.disk_gb) / DEFAULTS.storageMonthHours;
  const computeStorage = offer.dph_base + storage;
  const bandwidth =
    profile.bandwidth_down_gb * offer.inet_down_cost + profile.bandwidth_up_gb * offer.inet_up_cost;
  const total = (computeStorage * (hard + cronPeriod)) / 3600000 + bandwidth;
  return Number.isFinite(total) ? total : null;
}

// ---- rotation ------------------------------------------------------------------------------------------------------------
// Table: { release_seq, last: { image: { family: release_seq of the last run } } }.
const lastRun = (table, image, family) => {
  const v = table?.last?.[image]?.[family];
  return Number.isSafeInteger(v) ? v : undefined;
};
// The (image, family) pairs one release runs. Forced in: every pair whose last run is `windowReleases` or more releases
// old. Then each image runs its oldest pairs until it has ceil(families / windowReleases) in this release (so every image
// runs every release, and the load is level across images). Never-run pairs are the oldest. With every release recorded,
// each image meets every family within windowReleases releases (test/app-lane.test.js simulates several shapes).
export function selectRotation({ images, families, table, releaseSeq, windowReleases }) {
  if (!posInt(windowReleases) || !Number.isSafeInteger(releaseSeq))
    throw Error('rotation arguments');
  const pairs = images.flatMap((image) => families.map((family) => ({ image, family })));
  const age = (p) => {
    const last = lastRun(table, p.image, p.family);
    return last === undefined ? Infinity : releaseSeq - last;
  };
  const oldest = [...pairs].sort((a, b) => age(b) - age(a) || pairs.indexOf(a) - pairs.indexOf(b));
  const chosen = oldest.filter((p) => Number.isFinite(age(p)) && age(p) >= windowReleases);
  const perImage = Math.ceil(families.length / windowReleases);
  for (const image of images) {
    let count = chosen.filter((c) => c.image === image).length;
    for (const p of oldest) {
      if (count >= perImage) break;
      if (p.image === image && !chosen.includes(p)) {
        chosen.push(p);
        count++;
      }
    }
  }
  return chosen;
}
// The table after a release ran `legs` (each with image and family) at `releaseSeq`. Never mutates its input.
export function recordRotation(table, legs, releaseSeq) {
  const last = structuredClone(table?.last ?? {});
  for (const l of legs) (last[l.image] ??= {})[l.family] = releaseSeq;
  return { release_seq: releaseSeq, last };
}

// ---- the plan ------------------------------------------------------------------------------------------------------------
// `allowed`: offers already evaluated under the release profile (src/offers.js, the Worker's own rejection rules).
// Options: apps (application names: the images are the bundles that hold them) or images (ids, default all configured), families (default the matrix's die families), firstRelease (every image
// on every family, the rotation table is ignored), rotation (the table, required when not firstRelease), releaseSeq,
// dailyCapUsd (required: the plan is refused when its estimate plus spentTodayUsd exceeds it), spentTodayUsd.
// A leg is one image on one family on one box: the cheapest allowed offer of any die of that family with the image's VRAM,
// disk and bandwidth price, never pinned to a die; two legs never share a machine.
export function planAppLane(allowed, cfg, opts = {}) {
  const config = opts.config ?? APP_LANE;
  const { dailyCapUsd, spentTodayUsd = 0, firstRelease = false, rotation } = opts;
  if (!(typeof dailyCapUsd === 'number' && Number.isFinite(dailyCapUsd) && dailyCapUsd > 0))
    throw Error('daily cap: dailyCapUsd must be a positive number');
  if (!(typeof spentTodayUsd === 'number' && Number.isFinite(spentTodayUsd) && spentTodayUsd >= 0))
    throw Error('spentTodayUsd must be a non-negative number');
  if (!firstRelease && !isObject(rotation))
    throw Error('rotation: a rotation table is required unless firstRelease is set');
  const all = config.images;
  if (opts.apps !== undefined && opts.images !== undefined)
    throw Error('apps and images are alternatives: give one');
  // `apps`: the images that hold these applications (bundlesFor: fewest bytes), each of which must be an app-lane image.
  const wanted =
    opts.apps !== undefined
      ? bundlesFor(BUNDLES, opts.apps)
      : (opts.images ?? all.map((i) => i.id));
  for (const id of wanted) if (!all.some((i) => i.id === id)) throw Error('unknown image: ' + id);
  const images = all.filter((i) => wanted.includes(i.id));
  const families = opts.families ?? (cfg.releaseMatrix ?? RELEASE_MATRIX).policy.die_families;
  const windowReleases = config.rotation.window_releases;
  const releaseSeq = opts.releaseSeq ?? (rotation?.release_seq ?? 0) + 1;
  const pairs = firstRelease
    ? images.flatMap((i) => families.map((family) => ({ image: i.id, family })))
    : selectRotation({
        images: images.map((i) => i.id),
        families,
        table: rotation,
        releaseSeq,
        windowReleases,
      });
  const role = config.roles.image;
  const cost = { hard: role.hard_minutes * 60000, cronPeriod: cfg.cronPeriod };
  // Candidates per pair, cheapest first.
  const cands = pairs.map((pair) => {
    const image = images.find((i) => i.id === pair.image);
    const profile = imageProfile(image, opts.bandwidth);
    const list = [];
    for (const o of allowed) {
      if (familyOf(o.die, cfg) !== pair.family) continue;
      if (!(typeof o.gpu_ram === 'number' && o.gpu_ram >= profile.vram_gb * 1024)) continue;
      if (!(typeof o.disk_space === 'number' && o.disk_space >= profile.disk_gb)) continue;
      if (
        !(
          o.inet_down_cost <= role.max_inet_usd_per_gb && o.inet_up_cost <= role.max_inet_usd_per_gb
        )
      )
        continue;
      const total = appLegTotal(o, profile, cost);
      if (total !== null) list.push({ offer: o, total });
    }
    list.sort((a, b) => a.total - b.total || a.offer.id - b.offer.id);
    return { pair, image, profile, list };
  });
  // The scarcest pair picks first; a machine is used by one leg only.
  const used = new Set();
  const placed = new Map();
  for (const c of [...cands].sort((a, b) => a.list.length - b.list.length)) {
    const pick = c.list.find((x) => !used.has(x.offer.machine_id));
    if (!pick) continue;
    used.add(pick.offer.machine_id);
    placed.set(c, pick);
  }
  const legs = [],
    unplaced = [];
  for (const c of cands) {
    const pick = placed.get(c);
    if (!pick) {
      unplaced.push({ image: c.pair.image, family: c.pair.family, reason: 'NO_OFFER' });
      continue;
    }
    const o = pick.offer;
    legs.push({
      kind: 'app',
      image: c.pair.image,
      family: c.pair.family,
      die: o.die,
      offer_id: o.id,
      machine_id: o.machine_id,
      host_id: o.host_id ?? null,
      gpu_name: String(o.gpu_name).toUpperCase().replaceAll(' ', '_'),
      country: o.country,
      dph_total: round(o.dph_total),
      disk_gb: c.profile.disk_gb,
      vram_gb: c.profile.vram_gb,
      bandwidth_down_gb: c.profile.bandwidth_down_gb,
      bandwidth_up_gb: c.profile.bandwidth_up_gb,
      stages: c.profile.stages,
      manual_only: c.profile.manual_only,
      ...(c.profile.manual_only ? { only: [...c.profile.stages] } : {}),
      hard_minutes: role.hard_minutes,
      est_usd: round(pick.total),
    });
  }
  const estimate = round(legs.reduce((sum, l) => sum + l.est_usd, 0));
  const parallel = Math.min(cfg.maxConcurrentProfile.release, cfg.maxConcurrent);
  const refusals = [];
  if (unplaced.length) refusals.push('INCOMPLETE');
  if (parallel < 1) refusals.push('NO_SLOTS');
  if (estimate + spentTodayUsd > dailyCapUsd) refusals.push('DAILY_CAP');
  return {
    kind: 'app',
    mode: firstRelease ? 'first_release' : 'rotation',
    release_seq: releaseSeq,
    window_releases: windowReleases,
    families,
    images: images.map((i) => i.id),
    legs,
    unplaced,
    parallel,
    estimate_usd: estimate,
    spent_today_usd: spentTodayUsd,
    daily_cap_usd: dailyCapUsd,
    cost_model: { hard_ms: cost.hard, cron_ms: cost.cronPeriod },
    rotation_after: recordRotation(rotation ?? {}, legs, releaseSeq),
    admitted: refusals.length === 0,
    refusals,
  };
}
