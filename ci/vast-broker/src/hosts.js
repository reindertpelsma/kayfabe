// Custom hosts (enrolled boxes that are not rented from Vast): the PURE parts. Contract:
// docs/design/V3_CUSTOM_HOSTS.md (STATUS: DESIGN). Nothing here does I/O except probeSeeder, which
// takes its fetcher as an argument, so no test depends on the network.
//
// Owner decisions encoded here: the profile is assigned by the admin, never by the box; a
// `borrowed` box only ever receives a declarative assignment (repo, commit, tree key, stage names
// from a fixed allowlist); commands exist for `owned` boxes only; a box never supplies an
// address the Worker contacts.
import { configuration, gpuClass } from './policy.js';
import { familyOf, normalTier } from './requirements.js';
import { constantTimeEqual, sha256Hex } from './tokens.js';

// owned: full access, driver swaps, remote commands (only inside an admin window). container: a
// container with /dev/kvm and /dev/nvidia*, host driver fixed. standard: an enrolled throwaway box
// rented elsewhere, treated exactly like a Vast box (the default for a new box). borrowed: distrusts
// us (PARKED pending an owner decision, see the BORROWED section below): declarative assignments
// only, and it never CONSUMES artifacts from other boxes.
export const PROFILES = Object.freeze(['owned', 'container', 'standard', 'borrowed']);
export const DEFAULT_PROFILE = 'standard';
// Wire values (golden fixture: test/fixtures/hosts-wire.json; the daemon copies it).
export const CANONICAL_REPO = 'https://github.com/reindertpelsma/kayfabe';
export const REPO_ALLOWLIST = Object.freeze([CANONICAL_REPO]);
export const WIRE_STAGES = Object.freeze([
  'provision',
  'cuda_host',
  'merge_bar',
  'cuda_guest',
  'apps',
  'windows_gate',
  'windows_apps',
  'specviewperf',
  'ladder',
]);
export const COMMANDS_MAX_MS = 4 * 3600 * 1000;
export const HOST_TIERS = Object.freeze(['small', 'middle', 'large']);
export const BOX_ID = /^[a-z0-9][a-z0-9-]{0,39}$/;
// Heartbeat every 15 s from the daemon; a box that has not been heard from for 45 s is gone.
export const HEARTBEAT_S = 15;
export const LIVENESS_MS = 45000;
// An offer that is not acked or nacked within this long is treated as lost.
export const ACK_TIMEOUT_MS = 30000;
// Content of probe.txt on the box (W3's nginx template serves exactly this).
export const SEED_PROBE_CONTENT = 'kfseed-probe-v1\n';
export const PROBE_FILE = 'probe.txt';
export class HostError extends Error {
  constructor(code, status = 400) {
    super(code);
    this.code = code;
    this.status = status;
  }
}
const bad = (code, status = 400) => {
  throw new HostError(code, status);
};
const isObject = (v) => v && typeof v === 'object' && !Array.isArray(v);
const onlyKeys = (value, allowed, code) => {
  if (!isObject(value) || Object.keys(value).some((k) => !allowed.includes(k))) bad(code);
};

// ---- addresses -------------------------------------------------------------------------------
// Returns the normalised literal (IPv6 without brackets) or null. Private, loopback, link-local,
// multicast and unspecified space is refused: the Worker contacts only public addresses.
export function publicIp(value) {
  if (typeof value !== 'string' || value.length > 64) return null;
  const v4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(value);
  if (v4) {
    const o = v4.slice(1).map(Number);
    if (o.some((x, i) => x > 255 || String(x) !== v4[i + 1])) return null;
    const [a, b, c] = o;
    if (
      a === 0 ||
      a === 10 ||
      a === 127 ||
      a >= 224 ||
      (a === 100 && b >= 64 && b <= 127) ||
      (a === 169 && b === 254) ||
      (a === 172 && b >= 16 && b <= 31) ||
      (a === 192 && b === 168) ||
      (a === 192 && b === 0 && c === 0) ||
      (a === 198 && (b === 18 || b === 19))
    )
      return null;
    return value;
  }
  if (!/^[0-9a-fA-F:.]+$/.test(value) || !value.includes(':')) return null;
  let host;
  try {
    host = new URL('http://[' + value + ']/').hostname;
  } catch {
    return null;
  }
  const inner = host.slice(1, -1).toLowerCase();
  if (
    inner === '::' ||
    inner === '::1' ||
    /^f[cd]/.test(inner) ||
    /^fe[89ab]/.test(inner) ||
    /^ff/.test(inner) ||
    inner.startsWith('::ffff:') ||
    inner.startsWith('64:ff9b:')
  )
    return null;
  return inner;
}
// IPv4 dotted quad only (no IPv6 for now), in public space.
export const publicIpv4 = (value) =>
  typeof value === 'string' && /^[0-9.]+$/.test(value) ? publicIp(value) : null;
// Workers cannot fetch() a raw IP (Cloudflare answers 403/1003, MEASURED 2026-11-12); a hostname that
// resolves to the IP works on a non-standard port. IP-encoding DNS mappers, in the order tried. This
// is the one place to add or drop a mapper. sslip.io and nip.io share nameservers (one operator),
// so nip.io is not an independent second path and is not listed; local-ip.sh works but is disabled.
// traefik.me through a real Worker fetch is not yet measured (integration).
export const MAPPERS = Object.freeze([
  Object.freeze({ zone: 'sslip.io', enabled: true }),
  Object.freeze({ zone: 'traefik.me', enabled: true }),
  Object.freeze({ zone: 'local-ip.sh', enabled: false }),
]);
// Names for a public dotted-quad, `<a-b-c-d>.<zone>` per enabled mapper; [] for anything else (IPv6,
// private, loopback, link-local, CGNAT, non-dotted-quad). The connection is untrusted anyway
// (consumers verify size and sha256), so there is no own-zone or admin-chosen name.
export function probeHostnames(ip) {
  const v4 = publicIpv4(ip);
  if (!v4) return [];
  return MAPPERS.filter((m) => m.enabled).map((m) => v4.replaceAll('.', '-') + '.' + m.zone);
}
const validPort = (p) => Number.isInteger(p) && p >= 1 && p <= 65535;

// ---- config ----------------------------------------------------------------------------------
const CONFIG_KEYS = [
  'enabled',
  'profile',
  'tiers',
  'seeder',
  'window',
  'opportunistic',
  'hourly_price_usd',
  'notes',
];
const HHMM = /^([01][0-9]|2[0-3]):([0-5][0-9])$/;
const minutesOf = (s) => Number(s.slice(0, 2)) * 60 + Number(s.slice(3));

// Validates an admin PUT body and returns the normalised config. Unknown keys are refused.
export function validateHostConfig(body) {
  onlyKeys(body, CONFIG_KEYS, 'CONFIG_KEYS');
  if (typeof body.enabled !== 'boolean') bad('CONFIG_ENABLED');
  const profile = body.profile === undefined ? DEFAULT_PROFILE : body.profile;
  if (!PROFILES.includes(profile)) bad('CONFIG_PROFILE');
  if (
    !Array.isArray(body.tiers) ||
    body.tiers.length > 3 ||
    new Set(body.tiers).size !== body.tiers.length ||
    body.tiers.some((t) => !HOST_TIERS.includes(t))
  )
    bad('CONFIG_TIERS');
  const seederIn = body.seeder ?? { enabled: false };
  onlyKeys(seederIn, ['enabled', 'remote_ip', 'external_port', 'inner_port'], 'CONFIG_SEEDER');
  if (typeof seederIn.enabled !== 'boolean') bad('CONFIG_SEEDER');
  const seeder = { enabled: seederIn.enabled };
  if (seederIn.enabled) {
    const ip = publicIpv4(seederIn.remote_ip);
    if (!ip) bad('CONFIG_SEEDER_IP');
    if (!validPort(seederIn.external_port) || !validPort(seederIn.inner_port))
      bad('CONFIG_SEEDER_PORT');
    Object.assign(seeder, {
      remote_ip: ip,
      external_port: seederIn.external_port,
      inner_port: seederIn.inner_port,
    });
  }
  let window = null;
  if (body.window !== undefined && body.window !== null) {
    onlyKeys(body.window, ['start', 'end'], 'CONFIG_WINDOW');
    const { start, end } = body.window;
    if (!HHMM.test(start ?? '') || !HHMM.test(end ?? '') || start === end) bad('CONFIG_WINDOW');
    window = { start, end };
  }
  if (body.opportunistic !== undefined && typeof body.opportunistic !== 'boolean')
    bad('CONFIG_OPPORTUNISTIC');
  // Optional hourly price (a rented box from another provider): orders selection only.
  const price = body.hourly_price_usd ?? null;
  if (
    price !== null &&
    !(typeof price === 'number' && Number.isFinite(price) && price >= 0 && price <= 50)
  )
    bad('CONFIG_PRICE');
  if (
    body.notes !== undefined &&
    (typeof body.notes !== 'string' ||
      body.notes.length > 500 ||
      /[\u0000-\u0008\u000b-\u001f]/.test(body.notes))
  )
    bad('CONFIG_NOTES');
  return {
    enabled: body.enabled,
    profile,
    tiers: [...body.tiers].sort(),
    seeder,
    window,
    opportunistic: body.opportunistic === true,
    hourly_price_usd: price,
    notes: body.notes ?? '',
  };
}

// The availability window is in UTC. null means always. start > end wraps over midnight.
export function inWindow(window, now) {
  if (!window) return true;
  const d = new Date(now),
    t = d.getUTCHours() * 60 + d.getUTCMinutes(),
    s = minutesOf(window.start),
    e = minutesOf(window.end);
  return s < e ? t >= s && t < e : t >= s || t < e;
}

// ---- hello capabilities ------------------------------------------------------------------------
const STR = (v) => typeof v === 'string' && v.length <= 128 && !/[\u0000-\u001f]/.test(v);
const NUM = (v) => typeof v === 'number' && Number.isFinite(v) && v >= 0 && v < 1e9;
const CAP_FIELDS = {
  gpu_name: STR,
  compute_cap: (v) => typeof v === 'string' && /^[0-9]{1,2}\.[0-9]$/.test(v),
  vram_gb: NUM,
  driver: STR,
  module_flavor: (v) => v === 'open' || v === 'proprietary',
  kvm: (v) => typeof v === 'boolean',
  cpu_model: STR,
  cores: NUM,
  ram_gb: NUM,
  disk_free_gb: NUM,
  download_mbps: NUM,
  display: (v) => typeof v === 'boolean',
  can_swap_driver: (v) => typeof v === 'boolean',
  container: (v) => typeof v === 'boolean',
  os: STR,
  kernel: STR,
  // PCI device id, 4 lowercase hex digits (informational; the die is derived from name + cc).
  pci_device_id: (v) => typeof v === 'string' && /^[0-9a-f]{4}$/.test(v),
};
// Keeps only known, well-typed fields; the box's claims are data, never authority.
export function sanitizeCapabilities(caps) {
  const out = {};
  if (!isObject(caps)) return out;
  for (const [k, ok] of Object.entries(CAP_FIELDS)) if (k in caps && ok(caps[k])) out[k] = caps[k];
  return out;
}

// ---- BORROWED (parked: the owner has not decided whether the first merge carries this mode) ---------
// Everything that is specific to the `borrowed` (distrust us) profile is in this section and in the
// `isBorrowed`, `BORROWED_ASSIGNMENT_KEYS` and `seedAllowed` uses elsewhere, so it can move to a
// follow-up branch in one step: a borrowed box gets a declarative assignment only (no env, no
// requirements) and never consumes artifacts from another box. Owner ruling 2026-11-12: all other
// hosts trust each other for artifacts (they cannot affect the release artifact or reach private R2).
export const isBorrowed = (profile) => profile === 'borrowed';
export const BORROWED_ASSIGNMENT_KEYS = Object.freeze(['repo', 'commit', 'tree_key', 'stages']);
export const seedAllowed = (consumerProfile) => !isBorrowed(consumerProfile);

// A git-only box (profile borrowed, or a registry snapshot that reports enforced.trust_git_only; the
// flags model itself is not built) must never be scheduled a commit the Worker does not KNOW is in
// master or next history. The knowledge is the verified OIDC claims of the run (the existing
// admission code validates them): event_name `push` on refs/heads/master or refs/heads/next. Any
// other trigger (pull_request, workflow_dispatch from another ref, a requirement built for a PR
// head) is not eligible, and so is a run whose claims are missing (fail closed). No GitHub call is
// made here. Boxes that are not git-only have no commit restriction.
export const COMMIT_SKIP = 'COMMIT_NOT_IN_MAIN_HISTORY';
export const MAIN_REFS = Object.freeze(['refs/heads/master', 'refs/heads/next']);
export const isGitOnly = (snap) =>
  isBorrowed(snap?.config?.profile) || snap?.enforced?.trust_git_only === true;
export function commitEligible(snap, run) {
  if (!isGitOnly(snap)) return true;
  return !!run && run.event_name === 'push' && MAIN_REFS.includes(run.ref);
}

// ---- profile enforcement -----------------------------------------------------------------------
const COMMIT = /^[0-9a-f]{40}$/;
const TREE_KEY = /^[0-9a-f]{64}$/;
const ENV_NAME = /^KF_[A-Z0-9_]{1,60}$/;

// Validates the assignment of an offer for the profile and returns a normalised copy. `borrowed`
// gets exactly repo/commit/tree_key/stages; no env, no requirements, nothing that could be read as a
// script or command. EVERY profile (owned too) gets the canonical repo URL from the allowlist
// (default REPO_ALLOWLIST), never an arbitrary URL, and stage names from WIRE_STAGES.
export function validateAssignment(profile, assignment, repoAllowlist = REPO_ALLOWLIST) {
  if (!PROFILES.includes(profile)) bad('OFFER_PROFILE');
  const allowed = isBorrowed(profile)
    ? BORROWED_ASSIGNMENT_KEYS
    : ['repo', 'commit', 'tree_key', 'stages', 'requirements', 'env'];
  onlyKeys(assignment, allowed, 'OFFER_ASSIGNMENT_KEYS');
  if (!repoAllowlist.includes(assignment.repo)) bad('OFFER_REPO');
  if (!COMMIT.test(assignment.commit ?? '')) bad('OFFER_COMMIT');
  if (!TREE_KEY.test(assignment.tree_key ?? '')) bad('OFFER_TREE_KEY');
  const out = { repo: assignment.repo, commit: assignment.commit, tree_key: assignment.tree_key };
  if (assignment.stages !== undefined) {
    const s = assignment.stages;
    if (!Array.isArray(s) || s.length > 32 || s.some((x) => typeof x !== 'string'))
      bad('OFFER_STAGES');
    if (s.some((x) => !WIRE_STAGES.includes(x))) bad('OFFER_STAGES');
    out.stages = [...s];
  }
  if (assignment.requirements !== undefined) {
    // Opaque to the daemon; bounded JSON of plain data.
    if (!isObject(assignment.requirements) || JSON.stringify(assignment.requirements).length > 4096)
      bad('OFFER_REQUIREMENTS');
    out.requirements = assignment.requirements;
  }
  if (assignment.env !== undefined) {
    const e = assignment.env;
    if (
      !isObject(e) ||
      Object.keys(e).length > 20 ||
      Object.entries(e).some(
        ([k, v]) => !ENV_NAME.test(k) || typeof v !== 'string' || !/^[\x20-\x7e]{0,256}$/.test(v),
      )
    )
      bad('OFFER_ENV');
    out.env = e;
  }
  return out;
}

// Remote commands need BOTH the owned profile and an admin-set window: commands_until (epoch ms,
// at most COMMANDS_MAX_MS from the moment it is set, default none). Unset or expired => refused.
export function validateCommandsUntil(value, now) {
  if (value === null) return null;
  if (!Number.isSafeInteger(value) || value <= now || value > now + COMMANDS_MAX_MS)
    bad('COMMANDS_UNTIL');
  return value;
}
export const commandsOpen = (config, now) =>
  config?.profile === 'owned' &&
  Number.isSafeInteger(config.commands_until) &&
  now < config.commands_until;

// Remote commands: `owned` only. Exactly one of argv (array of strings) or shell (string).
export function validateCommand(profile, body) {
  if (profile !== 'owned') bad('CMD_NOT_ALLOWED', 403);
  onlyKeys(body, ['argv', 'shell'], 'CMD_SHAPE');
  const hasArgv = body.argv !== undefined,
    hasShell = body.shell !== undefined;
  if (hasArgv === hasShell) bad('CMD_SHAPE');
  if (hasArgv) {
    if (
      !Array.isArray(body.argv) ||
      body.argv.length < 1 ||
      body.argv.length > 64 ||
      body.argv.some((a) => typeof a !== 'string' || a.length > 4096 || a.includes('\0'))
    )
      bad('CMD_SHAPE');
    return { argv: body.argv };
  }
  if (
    typeof body.shell !== 'string' ||
    !body.shell ||
    body.shell.length > 8192 ||
    body.shell.includes('\0')
  )
    bad('CMD_SHAPE');
  return { shell: body.shell };
}

// ---- selection (pure) ----------------------------------------------------------------------------
// A snapshot is what the registry reports per box:
//   {box_id, config, capabilities, connected, state: 'idle'|'busy', last_heartbeat, offer_pending}
// requirements: {tier, family?, die?, min_vram_gb?, driver?, needs_driver_swap?, kvm?, display?}.
// Returns the reason a box does not qualify, or null.
export function hostRejection(requirements, snap, now) {
  const cfg = snap.config,
    caps = snap.capabilities ?? {};
  const tier = normalTier(requirements.tier);
  if (!tier) return 'REQ_TIER';
  const hostTier = tier === 'largest' ? 'large' : tier;
  if (!cfg?.enabled) return 'DISABLED';
  if (!snap.connected || !(now - snap.last_heartbeat < LIVENESS_MS)) return 'NOT_CONNECTED';
  if (snap.state !== 'idle' || snap.offer_pending) return 'BUSY';
  if (!cfg.tiers.includes(hostTier)) return 'TIER';
  if (!inWindow(cfg.window, now)) return 'WINDOW';
  if (!commitEligible(snap, requirements.run)) return COMMIT_SKIP;
  const family =
    requirements.family && requirements.family !== 'any'
      ? requirements.family
      : requirements.die
        ? familyOf(requirements.die)
        : null;
  if (family && family !== 'unknown') {
    const cc = /^([0-9]{1,2})\.([0-9])$/.exec(caps.compute_cap ?? '');
    const have = cc ? familyOf('CC_' + cc[1] + cc[2]) : 'unknown';
    if (have !== family) return 'FAMILY';
  }
  // The die is derived by the Worker from the box's name and compute capability (dieOf); a box
  // without one never satisfies a die requirement.
  if (requirements.die && caps.die !== requirements.die) return 'DIE';
  if (requirements.min_vram_gb !== undefined && !(caps.vram_gb >= requirements.min_vram_gb))
    return 'VRAM';
  const swap = cfg.profile === 'owned' && caps.can_swap_driver === true;
  if (requirements.needs_driver_swap && !swap) return 'DRIVER_SWAP';
  if (requirements.driver && caps.driver !== requirements.driver && !swap) return 'DRIVER';
  if (requirements.kvm && caps.kvm !== true) return 'KVM';
  if (requirements.display && caps.display !== true) return 'DISPLAY';
  return null;
}

// Why each box was or was not chosen, for the admin and debug output: [{box_id, reason}] with
// reason null for a qualifying box.
export const explainHosts = (requirements, snapshots, now) =>
  snapshots.map((s) => ({ box_id: s.box_id, reason: hostRejection(requirements, s, now) }));

// Qualifying custom hosts, best first: non-opportunistic before opportunistic, then the cheaper
// hourly price first (a box with no price sorts after every priced one), then box id. The planner
// tries these before any Vast rental; an empty result means "fall back to Vast". The price orders
// otherwise equal boxes only; it never makes a box qualify.
export function hostsFor(requirements, snapshots, now) {
  return snapshots
    .filter((s) => hostRejection(requirements, s, now) === null)
    .sort(
      (a, b) =>
        Number(a.config.opportunistic) - Number(b.config.opportunistic) ||
        (a.config.hourly_price_usd ?? Infinity) - (b.config.hourly_price_usd ?? Infinity) ||
        (a.box_id < b.box_id ? -1 : a.box_id > b.box_id ? 1 : 0),
    )
    .map((s) => ({
      box_id: s.box_id,
      profile: s.config.profile,
      opportunistic: s.config.opportunistic,
      hourly_price_usd: s.config.hourly_price_usd ?? null,
    }));
}

// ---- die from capabilities (fail closed) ------------------------------------------------------
// The die is NEVER a box claim: it is derived through the existing die tables (policy.js gpuClass,
// dieRules in defaults.json) from gpu_name + compute_cap. Unknown names give CC_<cc>, which only a
// CC_ requirement matches. Returns undefined when nothing can be derived.
export function dieOf(caps, cfg) {
  const m = /^([0-9]{1,2})\.([0-9])$/.exec(caps?.compute_cap ?? '');
  if (!m || typeof caps.gpu_name !== 'string') return undefined;
  try {
    return (
      gpuClass(
        { gpu_arch: 'nvidia', compute_cap: Number(m[1] + m[2]) * 10, gpu_name: caps.gpu_name },
        cfg,
      ) ?? undefined
    );
  } catch {
    return undefined;
  }
}
export const dieOfEnv = (caps, env) => {
  try {
    return dieOf(caps, configuration(env));
  } catch {
    return undefined;
  }
};

// ---- seeds ---------------------------------------------------------------------------------------
// Owner ruling 2026-11-12: every host used for a CI GPU test is untrusted by design and none can
// affect the release artifact or reach private R2, so all hosts trust each other for artifacts.
// The one exception: a `borrowed` host distrusts us and NEVER consumes an artifact from another box;
// it builds from public sources only (it may still seed to others). The seeder's sha256 and size are
// labelled source=seeder_reported (integrity of the transfer only); a record the Worker pinned
// independently is source=worker_pinned.
export function seedFor(record, consumerProfile) {
  return record && seedAllowed(consumerProfile) ? record : null;
}
// Checks a registration against independently known artifacts {name: {sha256, size}}. Returns the
// source label, or throws HostError('HASH_MISMATCH') / ('ARTIFACT_UNKNOWN').
export function checkKnownArtifact(known, artifact) {
  if (!known || !Object.keys(known).length) return 'seeder_reported';
  const k = known[artifact.name];
  if (!k) bad('ARTIFACT_UNKNOWN');
  if (k.sha256 !== artifact.sha256 || k.size !== artifact.size) bad('HASH_MISMATCH');
  return 'worker_pinned';
}
export function validateKnownArtifacts(value) {
  if (value === undefined) return undefined;
  if (!isObject(value) || Object.keys(value).length > 16) bad('OFFER_ARTIFACTS');
  for (const [name, a] of Object.entries(value)) {
    if (
      !SEED_NAME.test(name) ||
      !isObject(a) ||
      !/^[0-9a-f]{64}$/.test(a.sha256 ?? '') ||
      !Number.isSafeInteger(a.size) ||
      a.size < 1
    )
      bad('OFFER_ARTIFACTS');
  }
  return value;
}

// A custom-host result counts only for a tier the box is enabled for, under the profile and box the
// run token names. claims = verified run-token claims ({box, hp}); config = the box's config.
export function customHostResultCountable(claims, config, tier) {
  const t = normalTier(tier) === 'largest' ? 'large' : normalTier(tier);
  return !!(
    claims?.box &&
    config?.enabled &&
    config.box_id === claims.box &&
    config.profile === claims.hp &&
    t &&
    config.tiers.includes(t)
  );
}

// ---- wire frames (golden fixture: test/fixtures/hosts-wire.json) --------------------------------
const OFFER_ID = /^[0-9a-f]{16}$/;
const CMD_ID = /^[0-9a-f]{16}$/;
const RUN_ID = /^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/;
const str = (v, max) => typeof v === 'string' && v.length <= max;
const int = (v) => Number.isSafeInteger(v) && v >= 0;
const hasOnly = (f, keys) => Object.keys(f).every((k) => keys.includes(k));
// Returns null when the daemon->Worker frame is well formed, else a short code.
export function validateDaemonFrame(f) {
  if (!isObject(f) || typeof f.type !== 'string') return 'FRAME';
  if (f.v !== undefined && f.v !== 1) return 'VERSION';
  const only = (keys) => (hasOnly(f, ['type', 'v', ...keys]) ? null : 'KEYS');
  switch (f.type) {
    case 'hello':
      if (!BOX_ID.test(f.box_id ?? '') || !isObject(f.capabilities)) return 'HELLO';
      if (f.version !== undefined && !str(f.version, 64)) return 'HELLO';
      return only(['box_id', 'version', 'capabilities']);
    case 'heartbeat':
      if (f.state !== 'idle' && f.state !== 'busy') return 'HEARTBEAT';
      if (f.ts !== undefined && !int(f.ts)) return 'HEARTBEAT';
      if (f.run_id !== undefined && !RUN_ID.test(f.run_id)) return 'HEARTBEAT';
      return only(['ts', 'state', 'run_id']);
    case 'ack':
      return OFFER_ID.test(f.offer_id ?? '') ? only(['offer_id']) : 'ACK';
    case 'nack':
      if (!OFFER_ID.test(f.offer_id ?? '') || (f.reason !== undefined && !str(f.reason, 200)))
        return 'NACK';
      return only(['offer_id', 'reason']);
    case 'seed_register': {
      const a = f.artifact;
      if (
        !OFFER_ID.test(f.offer_id ?? '') ||
        !isObject(a) ||
        !SEED_NAME.test(a.name ?? '') ||
        !/^[0-9a-f]{64}$/.test(a.sha256 ?? '') ||
        !Number.isSafeInteger(a.size) ||
        a.size < 1 ||
        !hasOnly(a, ['name', 'sha256', 'size']) ||
        !SEED_PATH.test(f.path ?? '') ||
        SEED_PATH.exec(f.path)[2] !== a.name
      )
        return 'SEED';
      return only(['offer_id', 'artifact', 'path']);
    }
    case 'cmd_output':
      if (
        !CMD_ID.test(f.cmd_id ?? '') ||
        !int(f.seq) ||
        (f.stream !== 'stdout' && f.stream !== 'stderr') ||
        !str(f.data, 16384) ||
        (f.eof !== undefined && typeof f.eof !== 'boolean')
      )
        return 'CMD_OUTPUT';
      return only(['cmd_id', 'seq', 'stream', 'data', 'eof']);
    case 'cmd_done':
      if (!CMD_ID.test(f.cmd_id ?? '') || !Number.isInteger(f.rc)) return 'CMD_DONE';
      return only(['cmd_id', 'rc']);
    default:
      return 'UNKNOWN_TYPE';
  }
}
// Returns null when the Worker->daemon frame is well formed.
export function validateServerFrame(f) {
  if (!isObject(f) || typeof f.type !== 'string') return 'FRAME';
  if (f.v !== undefined && f.v !== 1) return 'VERSION';
  const only = (keys) => (hasOnly(f, ['type', 'v', ...keys]) ? null : 'KEYS');
  switch (f.type) {
    case 'welcome':
      if (!BOX_ID.test(f.box_id ?? '') || !PROFILES.includes(f.profile) || !int(f.heartbeat_s))
        return 'WELCOME';
      return only(['box_id', 'profile', 'heartbeat_s']);
    case 'offer':
      if (
        !OFFER_ID.test(f.offer_id ?? '') ||
        !str(f.request_id, 64) ||
        !RUN_ID.test(f.run_id ?? '') ||
        !str(f.run_token, 1024) ||
        !int(f.deadline_ms) ||
        !isObject(f.assignment)
      )
        return 'OFFER';
      try {
        validateAssignment('owned', f.assignment);
      } catch {
        return 'OFFER';
      }
      return only(['offer_id', 'request_id', 'run_id', 'run_token', 'assignment', 'deadline_ms']);
    case 'cancel':
      return OFFER_ID.test(f.offer_id ?? '') ? only(['offer_id']) : 'CANCEL';
    case 'seed_ack':
      if (typeof f.ok !== 'boolean' || (f.reason !== undefined && !str(f.reason, 64)))
        return 'SEED_ACK';
      return only(['ok', 'reason']);
    case 'cmd':
      if (!CMD_ID.test(f.cmd_id ?? '') || (f.argv === undefined) === (f.shell === undefined))
        return 'CMD';
      return only(['cmd_id', 'argv', 'shell']);
    case 'error':
      return str(f.code, 64) ? only(['code']) : 'ERROR';
    default:
      return 'UNKNOWN_TYPE';
  }
}

// Constant-time compare of two equal-length lowercase hex strings, synchronous (used where an await
// between the check and the accept would reopen a race with token rotation).
export function hexEqual(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string' || a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  return diff === 0;
}

// ---- per-box token -------------------------------------------------------------------------------
export const mintBoxToken = () => {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return (
    'kfh_' +
    btoa(String.fromCharCode(...bytes))
      .replace(/\+/g, '-')
      .replace(/\//g, '_')
      .replace(/=+$/, '')
  );
};
export const hashBoxToken = (token) => sha256Hex(token);
export async function boxTokenMatches(storedHash, provided) {
  if (typeof storedHash !== 'string' || typeof provided !== 'string') return false;
  if (!/^kfh_[A-Za-z0-9_-]{43}$/.test(provided)) return false;
  return constantTimeEqual(storedHash, await sha256Hex(provided));
}

// ---- seeder probe --------------------------------------------------------------------------------
export const SEED_TOKEN = /^[a-z0-9]{8}$/;
export const SEED_NAME = /^[A-Za-z0-9._-]{1,100}$/;
// The path a box registers: /<8-char token>/<artifact name>.
export const SEED_PATH = /^\/([a-z0-9]{8})\/([A-Za-z0-9._-]{1,100})$/;

export function seedUrl(ip, port, token, name) {
  const host = ip.includes(':') ? '[' + ip + ']' : ip;
  return 'http://' + host + ':' + port + '/' + token + '/' + name;
}

// The consumer's second form of the artifact URL: the hostname form. `host` is the name that passed
// the probe (a name known to work from Cloudflare's side); without one, the first mapper's name.
// For boxes behind http proxies, DNS proxies or NAT64 that only reach the hostname. null if the
// address is not a public IPv4.
export function seedFallbackUrl(ip, port, token, name, host) {
  const names = probeHostnames(ip);
  const chosen = host === undefined ? names[0] : names.includes(host) ? host : undefined;
  return chosen ? 'http://' + chosen + ':' + port + '/' + token + '/' + name : null;
}

async function readBounded(response, limit) {
  const reader = response.body?.getReader();
  if (!reader) return '';
  const parts = [];
  let size = 0;
  try {
    while (size <= limit) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      parts.push(value);
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  if (size > limit) return null;
  return new TextDecoder('utf-8', { fatal: false }).decode(await new Blob(parts).arrayBuffer());
}

// Fetches http://<name>:<port>/<token>/probe.txt for each name of probeHostnames(ip), in order, with
// the injected fetcher: ONE attempt per name, each hard-bounded by timeoutMs (<= 5 s: a closed port
// otherwise blocks ~19 s in Cloudflare). The first name that returns the exact probe text wins;
// redirects are never followed and count as a failure of that name. ip and port come from the
// caller's TRUSTED source (the Vast API's report for a Vast instance, the admin's config for a
// custom box), never from the box. A raw IP is never fetched. If every name fails the box is
// "cannot open a port" for the session. Returns {ok: true, hostname} or {ok: false, reason}
// (the reason of the last name).
export async function probeSeeder({
  fetcher,
  ip,
  port,
  token,
  expected = SEED_PROBE_CONTENT,
  timeoutMs = 5000,
}) {
  const names = probeHostnames(ip);
  if (!names.length) return { ok: false, reason: 'BAD_ADDRESS' };
  if (!validPort(port)) return { ok: false, reason: 'BAD_PORT' };
  if (typeof token !== 'string' || !SEED_TOKEN.test(token))
    return { ok: false, reason: 'BAD_TOKEN' };
  if (typeof fetcher !== 'function') return { ok: false, reason: 'NO_FETCHER' };
  const limit = Math.min(timeoutMs, 5000);
  let reason = 'UNREACHABLE';
  for (const hostname of names) {
    const url = 'http://' + hostname + ':' + port + '/' + token + '/' + PROBE_FILE;
    const controller = new AbortController();
    let timer;
    // The race also bounds a fetcher that ignores the abort signal.
    const deadline = new Promise((_, reject) => {
      timer = setTimeout(() => {
        controller.abort();
        reject(new Error('timeout'));
      }, limit);
    });
    try {
      const response = await Promise.race([
        fetcher(url, {
          method: 'GET',
          redirect: 'manual',
          signal: controller.signal,
          headers: { Accept: 'text/plain' },
        }),
        deadline,
      ]);
      if (response.type === 'opaqueredirect' || (response.status >= 300 && response.status < 400)) {
        reason = 'REDIRECT';
        continue;
      }
      if (response.status !== 200) {
        reason = 'HTTP_' + response.status;
        continue;
      }
      const body = await readBounded(response, 1024);
      if (body !== expected) {
        reason = 'WRONG_CONTENT';
        continue;
      }
      return { ok: true, hostname };
    } catch {
      reason = controller.signal.aborted ? 'TIMEOUT' : 'UNREACHABLE';
    } finally {
      clearTimeout(timer);
    }
  }
  return { ok: false, reason };
}
