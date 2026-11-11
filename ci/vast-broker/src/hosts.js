// Custom hosts (enrolled boxes that are not rented from Vast): the PURE parts. Contract:
// docs/design/V3_CUSTOM_HOSTS.md (STATUS: DESIGN). Nothing here does I/O except probeSeeder, which
// takes its fetcher as an argument, so no test depends on the network.
//
// Owner decisions encoded here: the profile is assigned by the admin, never by the box; a
// `borrowed` box only ever receives a declarative assignment (repo, commit, tree key, stage names
// from a fixed allowlist); commands exist for `owned` boxes only; a box never supplies an
// address the Worker contacts.
import { DEFAULTS } from './defaults.js';
import { familyOf, normalTier } from './requirements.js';
import { constantTimeEqual, sha256Hex } from './tokens.js';

export const PROFILES = Object.freeze(['owned', 'container', 'borrowed']);
export const HOST_TIERS = Object.freeze(['small', 'middle', 'large']);
export const BOX_ID = /^[a-z0-9][a-z0-9-]{0,39}$/;
// Heartbeat every 15 s from the daemon; a box that has not been heard from for 45 s is gone.
export const HEARTBEAT_S = 15;
export const LIVENESS_MS = 45000;
// An offer that is not acked or nacked within this long is treated as lost.
export const ACK_TIMEOUT_MS = 30000;
// Content of probe.txt on the box (W3's nginx template serves exactly this).
export const SEED_PROBE_CONTENT = 'kayfabe-seed-probe-v1\n';
export const PROBE_FILE = 'probe.txt';
// Stage names a `borrowed` offer may name: the stages the Worker itself knows (payload stages and
// stage slots in defaults.json). Anything else is refused, never forwarded.
export const BORROWED_STAGES = Object.freeze(
  [
    ...new Set([
      ...Object.keys(DEFAULTS.payloadStageResources ?? {}),
      ...Object.keys(DEFAULTS.stageSlots ?? {}),
    ]),
  ].sort(),
);

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
// resolves to the IP works on a non-standard port. Candidates, in order: <a-b-c-d>.sslip.io, then
// <a.b.c.d>.nip.io. Pure; IPv4 in public space only. The connection is untrusted anyway (consumers
// verify size and sha256), so there is no own-zone or admin-chosen name.
export function probeHostnames(ip) {
  const v4 = publicIpv4(ip);
  if (!v4) return [];
  return [v4.replaceAll('.', '-') + '.sslip.io', v4 + '.nip.io'];
}
const validPort = (p) => Number.isInteger(p) && p >= 1 && p <= 65535;

// ---- config ----------------------------------------------------------------------------------
const CONFIG_KEYS = ['enabled', 'profile', 'tiers', 'seeder', 'window', 'opportunistic', 'notes'];
const HHMM = /^([01][0-9]|2[0-3]):([0-5][0-9])$/;
const minutesOf = (s) => Number(s.slice(0, 2)) * 60 + Number(s.slice(3));

// Validates an admin PUT body and returns the normalised config. Unknown keys are refused.
export function validateHostConfig(body) {
  onlyKeys(body, CONFIG_KEYS, 'CONFIG_KEYS');
  if (typeof body.enabled !== 'boolean') bad('CONFIG_ENABLED');
  if (!PROFILES.includes(body.profile)) bad('CONFIG_PROFILE');
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
  if (
    body.notes !== undefined &&
    (typeof body.notes !== 'string' ||
      body.notes.length > 500 ||
      /[\u0000-\u0008\u000b-\u001f]/.test(body.notes))
  )
    bad('CONFIG_NOTES');
  return {
    enabled: body.enabled,
    profile: body.profile,
    tiers: [...body.tiers].sort(),
    seeder,
    window,
    opportunistic: body.opportunistic === true,
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
  die: (v) => typeof v === 'string' && /^[A-Z0-9_]{2,24}$/.test(v),
};
// Keeps only known, well-typed fields; the box's claims are data, never authority.
export function sanitizeCapabilities(caps) {
  const out = {};
  if (!isObject(caps)) return out;
  for (const [k, ok] of Object.entries(CAP_FIELDS)) if (k in caps && ok(caps[k])) out[k] = caps[k];
  return out;
}

// ---- profile enforcement -----------------------------------------------------------------------
const REPO = /^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})\/(?!\.{1,2}$)[A-Za-z0-9_.-]{1,100}$/;
const COMMIT = /^[0-9a-f]{40}$/;
const TREE_KEY = /^[A-Za-z0-9:_.-]{8,128}$/;
const ENV_NAME = /^KF_[A-Z0-9_]{1,60}$/;

// Validates the assignment of an offer for the profile and returns a normalised copy. `borrowed`
// gets exactly repo/commit/tree_key/stages, with stage names from BORROWED_STAGES; no env, no
// requirements, nothing that could be read as a script or command.
export function validateAssignment(profile, assignment) {
  if (!PROFILES.includes(profile)) bad('OFFER_PROFILE');
  const allowed =
    profile === 'borrowed'
      ? ['repo', 'commit', 'tree_key', 'stages']
      : ['repo', 'commit', 'tree_key', 'stages', 'requirements', 'env'];
  onlyKeys(assignment, allowed, 'OFFER_ASSIGNMENT_KEYS');
  if (!REPO.test(assignment.repo ?? '')) bad('OFFER_REPO');
  if (!COMMIT.test(assignment.commit ?? '')) bad('OFFER_COMMIT');
  if (!TREE_KEY.test(assignment.tree_key ?? '')) bad('OFFER_TREE_KEY');
  const out = { repo: assignment.repo, commit: assignment.commit, tree_key: assignment.tree_key };
  if (assignment.stages !== undefined) {
    const s = assignment.stages;
    if (!Array.isArray(s) || s.length > 32 || s.some((x) => typeof x !== 'string'))
      bad('OFFER_STAGES');
    if (
      profile === 'borrowed'
        ? s.some((x) => !BORROWED_STAGES.includes(x))
        : s.some((x) => !/^[a-z][a-z0-9_]{0,47}$/.test(x))
    )
      bad('OFFER_STAGES');
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
  if (requirements.die && caps.die && caps.die !== requirements.die) return 'DIE';
  if (requirements.min_vram_gb !== undefined && !(caps.vram_gb >= requirements.min_vram_gb))
    return 'VRAM';
  const swap = cfg.profile === 'owned' && caps.can_swap_driver === true;
  if (requirements.needs_driver_swap && !swap) return 'DRIVER_SWAP';
  if (requirements.driver && caps.driver !== requirements.driver && !swap) return 'DRIVER';
  if (requirements.kvm && caps.kvm !== true) return 'KVM';
  if (requirements.display && caps.display !== true) return 'DISPLAY';
  return null;
}

// Qualifying custom hosts, best first: non-opportunistic before opportunistic, then box id. The
// planner tries these before any Vast rental; an empty result means "fall back to Vast".
export function hostsFor(requirements, snapshots, now) {
  return snapshots
    .filter((s) => hostRejection(requirements, s, now) === null)
    .sort(
      (a, b) =>
        Number(a.config.opportunistic) - Number(b.config.opportunistic) ||
        (a.box_id < b.box_id ? -1 : a.box_id > b.box_id ? 1 : 0),
    )
    .map((s) => ({
      box_id: s.box_id,
      profile: s.config.profile,
      opportunistic: s.config.opportunistic,
    }));
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
const SEED_TOKEN = /^[A-Za-z0-9_-]{16,128}$/;
export const SEED_NAME = /^[A-Za-z0-9._-]{1,100}$/;

export function seedUrl(ip, port, token, name) {
  const host = ip.includes(':') ? '[' + ip + ']' : ip;
  return 'http://' + host + ':' + port + '/' + token + '/' + name;
}

// The consumer's second form of the artifact URL: the hostname form, built with the probe's own
// mapper (<a-b-c-d>.sslip.io; nip.io is the probe's retry name). For boxes
// behind http proxies, DNS proxies or NAT64 that only reach the hostname. null if no valid host.
export function seedFallbackUrl(ip, port, token, name) {
  const host = probeHostnames(ip)[0];
  return host ? 'http://' + host + ':' + port + '/' + token + '/' + name : null;
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

// Fetches http://<hostname>:<port>/<token>/probe.txt with the injected fetcher and compares the body
// exactly. The hostname is derived from ip (see probeHostnames); ip and port come from the caller's TRUSTED source (the
// Vast API's report for a Vast instance, the admin's config for a custom box), never from the box.
// Each attempt is hard-bounded by timeoutMs (<= 5 s: a closed port otherwise blocks ~19 s in
// Cloudflare); there is one retry, which uses the next candidate name when there is one. A redirect
// or a wrong body is final. A raw IP is never fetched. Returns {ok: true} or {ok: false, reason}.
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
  for (let attempt = 0; attempt < 2; attempt++) {
    const url =
      'http://' +
      names[Math.min(attempt, names.length - 1)] +
      ':' +
      port +
      '/' +
      token +
      '/' +
      PROBE_FILE;
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
      if (response.type === 'opaqueredirect' || (response.status >= 300 && response.status < 400))
        return { ok: false, reason: 'REDIRECT' };
      if (response.status !== 200) {
        reason = 'HTTP_' + response.status;
        continue;
      }
      const body = await readBounded(response, 1024);
      if (body !== expected) return { ok: false, reason: 'WRONG_CONTENT' };
      return { ok: true };
    } catch {
      reason = controller.signal.aborted ? 'TIMEOUT' : 'UNREACHABLE';
    } finally {
      clearTimeout(timer);
    }
  }
  return { ok: false, reason };
}
