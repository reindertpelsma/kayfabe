// HostBox: the Durable Object of one custom host (one instance per box id), WebSocket
// HIBERNATION API. Contract: docs/design/V3_CUSTOM_HOSTS.md (STATUS: DESIGN). Not deployed.
//
// The same class also serves, under the reserved instance name INDEX_NAME, the list of box ids
// (admin list and the planner's snapshots). Everything except the daemon's WebSocket upgrade is
// reachable only on the internal origin (the gateway, the admin Worker's DO binding, the planner).
//
// Frames are JSON objects, one per frame, {v: 1, type: <name>, ...}; the discriminator is `type`.
// `v` may be absent on daemon frames (the daemon of PR #346 omits it); any other value is refused.
// Logs, progress, results and evidence do not travel here: the offer carries the per-run token and
// the box uses the existing HTTPS box protocol.
import {
  ACK_TIMEOUT_MS,
  BOX_ID,
  HEARTBEAT_S,
  HostError,
  LIVENESS_MS,
  REPO_ALLOWLIST,
  SEED_PATH,
  seedAllowed,
  validateSeedRef,
  checkKnownArtifact,
  commandsOpen,
  dieOfEnv,
  hashBoxToken,
  hexEqual,
  mintBoxToken,
  probeSeeder,
  sanitizeCapabilities,
  seedFallbackUrl,
  seedFor,
  seedUrl,
  validateAssignment,
  validateCommand,
  validateCommandsUntil,
  validateDaemonFrame,
  validateHostConfig,
  validateKnownArtifacts,
} from './hosts.js';
import { normalTier } from './requirements.js';
import { numericId } from './policy.js';
import { RunTokens } from './tokens.js';

export const INDEX_NAME = '__index__';
export const MAX_BOXES = 64;
export const MAX_FRAME = 65536;
export const MAX_COMMANDS = 20;
export const MAX_CMD_OUTPUT = 32768;
// After an ack, an idle heartbeat without a run id only ends the offer once this long has passed
// (the daemon reports busy from its next heartbeat on).
export const ACK_GRACE_MS = 2 * HEARTBEAT_S * 1000 + 5000;
const OPEN = 1;
// Storage and frame budgets (review M5): a heartbeat is written at most every 60 s unless the state
// changes; command output is flushed in batches; a socket gets a token bucket.
export const HB_STORE_MS = 60000;
export const CMD_FLUSH_BYTES = 4096;
export const CMD_FLUSH_MS = 1000;
export const BUCKET_CAPACITY = 20;
export const BUCKET_PER_S = 10;
// cmd_output frames get their own, larger bucket (a chatty command is paced by the daemon to this).
export const CMD_OUTPUT_BURST = 40;
export const CMD_OUTPUT_PER_S = 20;
// A coarse bucket in front of the JSON parse, for every frame.
export const RAW_BURST = 80;
export const RAW_PER_S = 40;
const BEARER = /^Bearer (kfh_[A-Za-z0-9_-]{43})$/;

const json = (data, status = 200) =>
  Response.json(data, {
    status,
    headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
  });
const isObject = (v) => v && typeof v === 'object' && !Array.isArray(v);
const randomHex = (bytes) =>
  Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (x) =>
    x.toString(16).padStart(2, '0'),
  ).join('');

async function readJson(request, limit = 8192) {
  const reader = request.body?.getReader();
  if (!reader) throw new HostError('BODY', 400);
  const parts = [];
  let size = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > limit) throw new HostError('BODY_TOO_LARGE', 413);
      parts.push(value);
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  try {
    return JSON.parse(
      new TextDecoder('utf-8', { fatal: true }).decode(await new Blob(parts).arrayBuffer()),
    );
  } catch {
    throw new HostError('BODY', 400);
  }
}

export class HostBox {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
    // Injectable for tests; the seeder probe uses this fetcher and nothing else.
    this.fetcher = (...args) => fetch(...args);
    this.pending = new Map(); // cmd_id -> buffered output not yet flushed
    this.probing = new Map(); // offer_id -> {token, promise} of the probe in flight
    this.buckets = new WeakMap(); // ws -> token bucket
  }
  now() {
    return Date.now();
  }
  stub(id) {
    return this.env.HOSTS.get(this.env.HOSTS.idFromName(id));
  }
  get(key) {
    return this.ctx.storage.get(key);
  }
  put(key, value) {
    return this.ctx.storage.put(key, value);
  }
  repoAllowlist() {
    try {
      const v = JSON.parse(this.env.HOST_REPO_ALLOWLIST ?? 'null');
      if (Array.isArray(v) && v.length && v.every((x) => /^https:\/\/[^\s?#]+$/.test(x))) return v;
    } catch {}
    return REPO_ALLOWLIST;
  }
  openSockets() {
    return this.ctx.getWebSockets().filter((w) => w.readyState === OPEN);
  }
  closeAll(code, reason) {
    for (const w of this.ctx.getWebSockets()) {
      try {
        w.close(code, reason);
      } catch {}
    }
  }
  send(ws, frame) {
    ws.send(JSON.stringify({ v: 1, ...frame }));
  }
  // Sockets that completed hello and were heard from within LIVENESS_MS (per-socket attachment:
  // liveness costs no storage write).
  live(now) {
    return this.openSockets().filter((w) => {
      const a = w.deserializeAttachment();
      return a?.helloed && now - a.seen < LIVENESS_MS;
    });
  }
  // kind: 'raw' (before parsing, every frame), 'frame' (every frame but cmd_output), 'out' (cmd_output).
  bucket(ws, now, kind = 'frame') {
    const [cap, per] =
      kind === 'raw'
        ? [RAW_BURST, RAW_PER_S]
        : kind === 'out'
          ? [CMD_OUTPUT_BURST, CMD_OUTPUT_PER_S]
          : [BUCKET_CAPACITY, BUCKET_PER_S];
    let all = this.buckets.get(ws);
    if (!all) this.buckets.set(ws, (all = {}));
    const b = (all[kind] ??= { tokens: cap, at: now });
    b.tokens = Math.min(cap, b.tokens + ((now - b.at) / 1000) * per);
    b.at = now;
    if (b.tokens < 1) return false;
    b.tokens -= 1;
    return true;
  }

  // ---- views -------------------------------------------------------------------------------
  async state() {
    await this.flushCommands();
    const [config, caps, hb, offer, version, tokenHash, tokenAt, seed, commands, enforced] =
      await Promise.all(
        [
          'config',
          'caps',
          'hb',
          'offer',
          'version',
          'tokenHash',
          'tokenAt',
          'seed',
          'commands',
          'enforced',
        ].map((k) => this.get(k)),
      );
    return { config, caps, hb, offer, version, tokenHash, tokenAt, seed, commands, enforced };
  }
  offerPending(offer, now) {
    if (!offer || now >= offer.deadline_ms) return false;
    if (offer.status === 'offered') return now - offer.offeredAt < ACK_TIMEOUT_MS;
    return offer.status === 'acked' || offer.status === 'ready';
  }
  async snapshot() {
    const s = await this.state(),
      now = this.now();
    if (!s.config) return null;
    const sockets = this.live(now);
    const att = sockets.map((w) => w.deserializeAttachment()).sort((a, b) => b.seen - a.seen)[0];
    return {
      box_id: s.config.box_id,
      config: s.config,
      capabilities: s.caps ?? {},
      connected: sockets.length > 0,
      state: att ? att.state : 'idle',
      run_id: att?.run_id ?? null,
      idle_since: att?.state === 'idle' ? (att.idle_since ?? null) : null,
      last_heartbeat: att?.seen ?? s.hb?.ts ?? 0,
      offer_pending: this.offerPending(s.offer, now),
    };
  }
  async view() {
    const s = await this.state(),
      snap = await this.snapshot();
    if (!snap) return null;
    return {
      ...snap,
      version: s.version ?? null,
      token_set: typeof s.tokenHash === 'string',
      token_minted_at: s.tokenAt ?? null,
      offer: s.offer ? { ...s.offer } : null,
      seed: s.seed ?? null,
      // The daemon's own local flags from its last hello: read-only facts.
      enforced: s.enforced ?? null,
      commands: s.commands ?? [],
    };
  }

  // ---- HTTP (internal origin only; except the WebSocket upgrade, which the gateway forwards) ----
  async fetch(request) {
    const url = new URL(request.url);
    if (url.origin !== 'https://internal.invalid') return json({ error: 'NOT_FOUND' }, 404);
    try {
      return await this.route(request, url);
    } catch (e) {
      if (e instanceof HostError) return json({ error: e.code }, e.status);
      return json({ error: 'UNAVAILABLE' }, 503);
    }
  }
  async route(request, url) {
    const path = url.pathname,
      method = request.method;
    let m;
    if ((m = /^\/hosts\/([^/]+)\/ws$/.exec(path)) && BOX_ID.test(m[1]) && method === 'GET')
      return this.upgrade(request, m[1]);
    if (path === '/admin/hosts' && method === 'GET')
      return this.indexList((id) => '/admin/hosts/' + id, 'hosts');
    if (path === '/internal/snapshots' && method === 'GET')
      return this.indexList(() => '/internal/snapshot', 'snapshots');
    if (path === '/index/add' && method === 'POST') return this.indexAdd(await readJson(request));
    if (path === '/index/remove' && method === 'POST')
      return this.indexRemove(await readJson(request));
    if (
      (m = /^\/admin\/hosts\/([^/]+)(\/token|\/cmd|\/commands-until|\/enrol)?$/.exec(path)) &&
      BOX_ID.test(m[1])
    ) {
      const id = m[1];
      if (!m[2] && method === 'GET') {
        const v = await this.view();
        if (!v || v.box_id !== id) throw new HostError('NOT_FOUND', 404);
        return json(v);
      }
      if (!m[2] && method === 'PUT') return this.adminPut(id, await readJson(request));
      if (!m[2] && method === 'DELETE') return this.adminDelete(id);
      if (m[2] === '/token' && method === 'POST') return this.adminToken(id);
      if (m[2] === '/enrol' && method === 'POST') return this.adminEnrol(id);
      if (m[2] === '/cmd' && method === 'POST') return this.adminCmd(id, await readJson(request));
      if (m[2] === '/commands-until' && method === 'PUT')
        return this.adminCommandsUntil(id, await readJson(request));
    }
    if (path === '/internal/snapshot' && method === 'GET') {
      const snap = await this.snapshot();
      if (!snap) throw new HostError('NOT_FOUND', 404);
      return json(snap);
    }
    if (path === '/internal/seed' && method === 'GET') {
      // The consumer's profile is the planner's knowledge, never the box's.
      const consumer = url.searchParams.get('consumer');
      const record = await this.get('seed');
      if (!record) throw new HostError('NOT_FOUND', 404);
      const out = seedFor(record, consumer);
      if (!out) throw new HostError('SEED_NOT_ALLOWED', 403);
      return json(out);
    }
    if (path === '/internal/offer' && method === 'POST')
      return this.makeOffer(await readJson(request));
    if (path === '/internal/cancel' && method === 'POST')
      return this.cancelOffer(await readJson(request));
    return json({ error: 'NOT_FOUND' }, 404);
  }

  // ---- the daemon's socket ----------------------------------------------------------------------
  async upgrade(request, id) {
    if (request.headers.get('Upgrade') !== 'websocket')
      throw new HostError('UPGRADE_REQUIRED', 426);
    const [client, server] = Object.values(new WebSocketPair());
    const bearer = BEARER.exec(request.headers.get('Authorization') ?? '')?.[1];
    // The only await that is not a storage read comes FIRST; everything from the storage read to
    // acceptWebSocket is synchronous, so a token rotation cannot slip in between check and accept
    // (an input gate is closed while storage is awaited), and rotation closes every open socket.
    const provided = bearer ? await hashBoxToken(bearer) : null;
    const [config, storedHash, tokenGen] = await Promise.all([
      this.get('config'),
      this.get('tokenHash'),
      this.get('tokenGen'),
    ]);
    const reject = (code, reason) => {
      // A plain (non-hibernating) accept: the socket is closed at once, and workerd/miniflare
      // deliver an immediate close to the client only for such a socket.
      server.accept();
      server.close(code, reason);
      return new Response(null, { status: 101, webSocket: client });
    };
    // Unknown box, no token minted and a wrong token are indistinguishable (4401).
    if (!provided || !config || config.box_id !== id || !hexEqual(storedHash, provided))
      return reject(4401, 'bad token');
    if (!config.enabled) return reject(4403, 'box disabled');
    const now = this.now();
    // A live connection is never kicked by a newer one: the NEW one is refused. Only a dead or
    // silent socket is replaced.
    if (this.live(now).length) return reject(4409, 'a live connection exists');
    for (const old of this.ctx.getWebSockets()) {
      try {
        old.close(4409, 'replaced');
      } catch {}
    }
    this.ctx.acceptWebSocket(server);
    server.serializeAttachment({ helloed: false, gen: tokenGen ?? 0, seen: now, state: 'idle' });
    // The box must say hello within the liveness window or it is dropped.
    await this.ctx.storage.setAlarm(now + LIVENESS_MS);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, message) {
    if (ws.readyState !== OPEN) return;
    const now = this.now();
    if (typeof message !== 'string' || message.length > MAX_FRAME) return ws.close(1009, 'frame');
    if (!this.bucket(ws, now, 'raw')) return ws.close(1008, 'rate limit');
    let m;
    try {
      m = JSON.parse(message);
    } catch {
      return ws.close(1007, 'json');
    }
    const attachment = ws.deserializeAttachment() ?? {};
    if (!this.bucket(ws, now, m?.type === 'cmd_output' ? 'out' : 'frame'))
      return ws.close(1008, 'rate limit');
    const problem = validateDaemonFrame(m);
    if (problem === 'FRAME' || problem === 'VERSION' || (problem && !attachment.helloed))
      return ws.close(1008, 'frame');
    const [config, tokenGen] = await Promise.all([this.get('config'), this.get('tokenGen')]);
    // Defence in depth against a rotation race: a socket of an older token generation is dropped.
    if ((tokenGen ?? 0) !== attachment.gen) return ws.close(4401, 'token rotated');
    if (!config?.enabled) return ws.close(4403, 'box disabled');
    if (problem === 'UNKNOWN_TYPE') return this.send(ws, { type: 'error', code: 'UNKNOWN_TYPE' });
    if (problem) return this.send(ws, { type: 'error', code: 'BAD_FRAME' });
    if (m.type === 'hello') {
      if (attachment.helloed || m.box_id !== config.box_id) return ws.close(1008, 'hello');
      const caps = sanitizeCapabilities(m.capabilities);
      // The die comes from the Worker's own tables, never from the box.
      const die = dieOfEnv(caps, this.env);
      if (die) caps.die = die;
      await this.put('caps', caps);
      // Read-only facts about the daemon's own local restrictions (the admin cannot edit them).
      await this.put('enforced', m.enforced ?? null);
      await this.put('version', typeof m.version === 'string' ? m.version.slice(0, 64) : null);
      await this.put('hb', { ts: now, state: 'idle' });
      ws.serializeAttachment({
        ...attachment,
        helloed: true,
        seen: now,
        state: 'idle',
        idle_since: now,
        stored: now,
      });
      this.send(ws, {
        type: 'welcome',
        box_id: config.box_id,
        profile: config.profile,
        heartbeat_s: HEARTBEAT_S,
        commands_until_ms: config.commands_until ?? null,
        cmd_output_per_s: CMD_OUTPUT_PER_S,
      });
      await this.ctx.storage.setAlarm(now + LIVENESS_MS);
      return;
    }
    if (!attachment.helloed) return ws.close(1008, 'hello first');
    switch (m.type) {
      case 'heartbeat':
        return this.onHeartbeat(ws, attachment, m, now);
      case 'ack':
      case 'ready':
      case 'nack':
        ws.serializeAttachment({ ...attachment, seen: now });
        return this.onAck(ws, m);
      case 'seed_register':
        ws.serializeAttachment({ ...attachment, seen: now });
        return this.onSeed(ws, m, config);
      case 'cmd_output':
      case 'cmd_done':
        if (config.profile !== 'owned') return ws.close(1008, 'commands are for owned boxes');
        ws.serializeAttachment({ ...attachment, seen: now });
        return this.onCommand(m, now);
    }
  }
  async onHeartbeat(ws, a, m, now) {
    const next = { ...a, seen: now, state: m.state, run_id: m.run_id };
    // idle_since: when the box last became idle (shown in the admin list).
    next.idle_since = m.state === 'idle' ? (a.state === 'idle' ? a.idle_since : now) : null;
    const changed = a.state !== m.state || a.run_id !== m.run_id;
    // Liveness lives in the attachment; storage is written on a change or once a minute.
    if (changed || now - (a.stored ?? 0) >= HB_STORE_MS) {
      next.stored = now;
      await this.put('hb', { ts: now, state: m.state, ...(m.run_id ? { run_id: m.run_id } : {}) });
    }
    ws.serializeAttachment(next);
    if (m.state === 'idle' && !m.run_id) {
      const offer = await this.get('offer');
      if (['acked', 'ready'].includes(offer?.status) && now - offer.ackedAt > ACK_GRACE_MS)
        await this.put('offer', { ...offer, status: 'done' });
    }
  }
  // ack = the offer is accepted (profile checks passed, before the fetch); ready = the commit is
  // fetched and verified; nack = refused, or failed after an ack (the run is released and the
  // requirement stays unmet: it is never recorded as a result).
  async onAck(ws, m) {
    const offer = await this.get('offer');
    const now = this.now();
    // An ack that arrives after the ack timeout (or after the box was marked lost) still counts
    // while the offer's deadline has not passed: a dropped connection must not lose it.
    const open = ['offered', 'lost'].includes(offer?.status);
    const running = ['acked', 'ready'].includes(offer?.status);
    if (
      !offer ||
      m.offer_id !== offer.offer_id ||
      now >= offer.deadline_ms ||
      !(m.type === 'ack' ? open : m.type === 'ready' ? offer.status === 'acked' : open || running)
    )
      return this.send(ws, { type: 'error', code: 'UNKNOWN_OFFER' });
    if (m.type === 'ack') await this.put('offer', { ...offer, status: 'acked', ackedAt: now });
    else if (m.type === 'ready')
      await this.put('offer', { ...offer, status: 'ready', readyAt: now });
    else
      await this.put('offer', {
        ...offer,
        status: running ? 'failed' : 'nacked',
        reason: typeof m.reason === 'string' ? m.reason.slice(0, 128) : '',
        ...(running ? { failedAt: now, unmet: true } : {}),
      });
  }
  async onSeed(ws, m, config) {
    const answer = (ok, reason) =>
      this.send(ws, { type: 'seed_ack', ok, ...(ok ? {} : { reason }) });
    const offer = await this.get('offer');
    const a = m.artifact;
    if (
      !offer ||
      !['acked', 'ready'].includes(offer.status) ||
      m.offer_id !== offer.offer_id ||
      this.now() >= offer.deadline_ms
    )
      return answer(false, 'BAD_REGISTRATION');
    if (!config.seeder?.enabled) return answer(false, 'SEEDER_DISABLED');
    // Where the Worker knows the artifact independently, the registration must match it.
    let source;
    try {
      source = checkKnownArtifact(offer.artifacts, a);
    } catch (e) {
      return answer(false, e.code ?? 'BAD_REGISTRATION');
    }
    const token = SEED_PATH.exec(m.path)[1];
    // At most ONE probe per offer: its result is cached on the offer and reused, and concurrent
    // registrations share the one probe that is in flight.
    let probe = offer.probe;
    if (probe && probe.token !== token) return answer(false, 'PROBE_USED');
    if (!probe) {
      let flight = this.probing.get(offer.offer_id);
      if (flight && flight.token !== token) return answer(false, 'PROBE_USED');
      if (!flight) {
        // Only the admin-configured address is contacted; nothing the box said selects it.
        flight = {
          token,
          promise: probeSeeder({
            fetcher: this.fetcher,
            ip: config.seeder.remote_ip,
            port: config.seeder.external_port,
            token,
          }).finally(() => this.probing.delete(offer.offer_id)),
        };
        this.probing.set(offer.offer_id, flight);
      }
      const result = await flight.promise;
      probe = { token, ok: result.ok, hostname: result.hostname, reason: result.reason };
      // The probe took seconds; the offer may have been replaced meanwhile (a new generation). A
      // stale probe never writes anything.
      const current = await this.get('offer');
      if (
        !current ||
        current.offer_id !== offer.offer_id ||
        current.generation !== offer.generation
      )
        return answer(false, 'STALE_OFFER');
      if (!current.probe) await this.put('offer', { ...current, probe });
    }
    if (!probe.ok) return answer(false, probe.reason);
    const current = await this.get('offer');
    if (!current || current.offer_id !== offer.offer_id || current.generation !== offer.generation)
      return answer(false, 'STALE_OFFER');
    const s = config.seeder;
    await this.put('seed', {
      offer_id: offer.offer_id,
      generation: offer.generation,
      // sha256 and size are the seeder's words about its own file (integrity of the transfer only)
      // unless the Worker pinned the artifact independently.
      source,
      seeder_profile: config.profile,
      url: seedUrl(s.remote_ip, s.external_port, token, a.name),
      fallback_url: seedFallbackUrl(s.remote_ip, s.external_port, token, a.name, probe.hostname),
      name: a.name,
      sha256: a.sha256,
      size: a.size,
      // The seeder's own notes about its staging (bounded, printable); shown to the admin.
      warnings: Array.isArray(m.warnings) ? m.warnings : [],
      registered_at: this.now(),
    });
    return answer(true);
  }

  // Command output is buffered and flushed in batches; output after done is ignored. Eviction of the
  // object loses the buffer, so the first buffered frame of a burst leaves a durable `dirty_since`
  // marker and an alarm flushes within CMD_FLUSH_MS; if the object is rebuilt while the marker is
  // set, the log is flagged `incomplete` instead of silently claiming a complete run.
  async cmdEntry(id) {
    let e = this.pending.get(id);
    if (e) return e;
    const stored = ((await this.get('commands')) ?? []).find((c) => c.cmd_id === id);
    if (!stored) return null;
    e = {
      text: '',
      truncated: false,
      rc: null,
      done: !!stored.done,
      dirty: false,
      lost: stored.dirty_since !== undefined,
      last: this.now(),
      bytes: stored.output.length,
    };
    this.pending.set(id, e);
    return e;
  }
  async scheduleAlarm(at) {
    const current = await this.ctx.storage.getAlarm();
    if (current === null || at < current) await this.ctx.storage.setAlarm(at);
  }
  async onCommand(m, now) {
    const e = await this.cmdEntry(m.cmd_id);
    if (!e || e.done) return;
    if (m.type === 'cmd_done') {
      e.done = true;
      e.rc = m.rc;
      e.dirty = true;
      return this.flushCommands();
    }
    const room = Math.max(0, MAX_CMD_OUTPUT - e.bytes);
    e.text += m.data.slice(0, room);
    e.bytes += Math.min(room, m.data.length);
    if (m.data.length > room) e.truncated = true;
    if (!e.dirty) {
      // First buffered frame of a burst: durable marker plus a flush alarm.
      e.dirty = true;
      const commands = (await this.get('commands')) ?? [];
      const c = commands.find((x) => x.cmd_id === m.cmd_id);
      if (c && c.dirty_since === undefined) {
        c.dirty_since = now;
        await this.put('commands', commands);
      }
      await this.scheduleAlarm(Date.now() + CMD_FLUSH_MS);
    }
    if (e.text.length >= CMD_FLUSH_BYTES || now - e.last >= CMD_FLUSH_MS)
      await this.flushCommands();
  }
  async flushCommands() {
    const dirty = [...this.pending].filter(([, e]) => e.dirty);
    if (!dirty.length) return;
    const commands = (await this.get('commands')) ?? [];
    for (const [id, e] of dirty) {
      const c = commands.find((x) => x.cmd_id === id);
      if (c && !c.done) {
        c.output = (c.output + e.text).slice(0, MAX_CMD_OUTPUT);
        if (e.truncated) c.truncated = true;
        if (e.lost) c.incomplete = true;
        delete c.dirty_since;
        if (e.done) {
          c.done = true;
          c.rc = e.rc;
        }
      }
      e.text = '';
      e.dirty = false;
      e.last = this.now();
    }
    await this.put('commands', commands);
  }
  async webSocketClose(ws, code, reason) {
    await this.flushCommands();
    try {
      ws.close(code >= 1000 && code < 5000 && code !== 1005 && code !== 1006 ? code : 1000, reason);
    } catch {}
  }
  async webSocketError(ws) {
    try {
      ws.close(1011, 'error');
    } catch {}
  }
  // Liveness: no frame for LIVENESS_MS closes the socket. Sockets carry their own last-seen time in
  // the attachment, so a heartbeat costs no storage write; the alarm reschedules itself.
  async alarm() {
    await this.flushCommands();
    const now = this.now();
    const sockets = this.openSockets();
    if (!sockets.length) return;
    const fresh = sockets
      .map((w) => w.deserializeAttachment()?.seen ?? 0)
      .filter((seen) => now - seen < LIVENESS_MS);
    if (fresh.length) {
      await this.ctx.storage.setAlarm(Math.min(...fresh) + LIVENESS_MS);
      return;
    }
    this.closeAll(4408, 'heartbeat timeout');
    const offer = await this.get('offer');
    if (offer?.status === 'offered') await this.put('offer', { ...offer, status: 'lost' });
  }

  // ---- admin ----------------------------------------------------------------------------------
  async indexList(pathOf, key) {
    const ids = (await this.get('ids')) ?? [];
    const out = [];
    await Promise.all(
      ids.map(async (id) => {
        try {
          const r = await this.stub(id).fetch('https://internal.invalid' + pathOf(id));
          if (r.ok) out.push(await r.json());
        } catch {}
      }),
    );
    out.sort((a, b) => (a.box_id < b.box_id ? -1 : 1));
    return json({ [key]: out });
  }
  async indexAdd({ id }) {
    if (!BOX_ID.test(id ?? '')) throw new HostError('BOX_ID', 400);
    const ids = (await this.get('ids')) ?? [];
    if (!ids.includes(id)) {
      if (ids.length >= MAX_BOXES) throw new HostError('TOO_MANY_HOSTS', 409);
      await this.put('ids', [...ids, id].sort());
    }
    return json({ ok: true });
  }
  async indexRemove({ id }) {
    const ids = (await this.get('ids')) ?? [];
    await this.put(
      'ids',
      ids.filter((x) => x !== id),
    );
    return json({ ok: true });
  }
  async registry(path, id) {
    const r = await this.stub(INDEX_NAME).fetch('https://internal.invalid' + path, {
      method: 'POST',
      body: JSON.stringify({ id }),
    });
    if (!r.ok) throw new HostError((await r.json()).error ?? 'UNAVAILABLE', r.status);
  }
  async adminPut(id, body) {
    const cfg = validateHostConfig(body);
    await this.registry('/index/add', id);
    const old = await this.get('config');
    // The command window belongs to the owned profile: it survives a config edit, not a profile change.
    const keep = old?.profile === cfg.profile ? (old.commands_until ?? null) : null;
    await this.put('config', {
      box_id: id,
      ...cfg,
      commands_until: keep,
      updated_at: this.now(),
    });
    if (!cfg.enabled) this.closeAll(4403, 'box disabled');
    // A profile change takes effect on the next connection; do not keep a stale grant.
    else if (old && old.profile !== cfg.profile) this.closeAll(4409, 'profile changed');
    return json(await this.view());
  }
  async adminCommandsUntil(id, body) {
    const config = await this.get('config');
    if (!config || config.box_id !== id) throw new HostError('NOT_FOUND', 404);
    if (!isObject(body) || Object.keys(body).join() !== 'commands_until')
      throw new HostError('COMMANDS_UNTIL', 400);
    if (config.profile !== 'owned') throw new HostError('CMD_NOT_ALLOWED', 403);
    const until = validateCommandsUntil(body.commands_until, this.now());
    await this.put('config', { ...config, commands_until: until });
    return json(await this.view());
  }
  async adminDelete(id) {
    const config = await this.get('config');
    if (!config || config.box_id !== id) throw new HostError('NOT_FOUND', 404);
    this.closeAll(4403, 'box deleted');
    await this.ctx.storage.deleteAll();
    await this.registry('/index/remove', id);
    return json({ ok: true });
  }
  // Mints a fresh per-box token, replacing the old one: the generation is bumped first, the hash is
  // stored (never the token), and every open socket of the box ends now.
  async rotateToken() {
    const token = mintBoxToken();
    await this.put('tokenGen', ((await this.get('tokenGen')) ?? 0) + 1);
    await this.put('tokenHash', await hashBoxToken(token));
    await this.put('tokenAt', this.now());
    this.closeAll(4401, 'token rotated');
    return token;
  }
  async adminToken(id) {
    const config = await this.get('config');
    if (!config || config.box_id !== id) throw new HostError('NOT_FOUND', 404);
    return json({ box_id: id, token: await this.rotateToken(), shown_once: true });
  }
  // One command to paste on a box that has no provider API: it writes the token to a 0600 file with
  // a shell builtin (never an argv value) and runs the bootstrap script of the PINNED commit. The
  // token appears only in this response; nothing is logged.
  async adminEnrol(id) {
    const config = await this.get('config');
    if (!config || config.box_id !== id) throw new HostError('NOT_FOUND', 404);
    const commit = this.env.HOST_DAEMON_COMMIT;
    if (typeof commit !== 'string' || !/^[0-9a-f]{40}$/.test(commit))
      throw new HostError('ENROL_UNAVAILABLE', 503);
    let origin;
    try {
      origin = new URL(this.env.WORKER_URL).origin;
    } catch {
      throw new HostError('ENROL_UNAVAILABLE', 503);
    }
    const prefix = /^[A-Za-z0-9_-]{32,128}$/.test(this.env.BROKER_PATH_PREFIX ?? '')
      ? '/' + this.env.BROKER_PATH_PREFIX
      : '';
    const token = await this.rotateToken();
    const repo = this.repoAllowlist()[0];
    if (!repo.startsWith('https://github.com/')) throw new HostError('ENROL_UNAVAILABLE', 503);
    const raw = repo.replace('https://github.com/', 'https://raw.githubusercontent.com/');
    // The public Access client id (a var; a placeholder until set). The Access client SECRET is never
    // known to the Worker: the admin writes it into ./accessfile before running the command.
    const accessId = /^[A-Za-z0-9._-]{8,128}$/.test(this.env.HOST_ACCESS_ID ?? '')
      ? this.env.HOST_ACCESS_ID
      : '<ACCESS_ID>';
    const command =
      '# First write the Cloudflare Access client SECRET (the Worker does not know it) into ./accessfile, ' +
      "e.g.: umask 077; printf '%s' '<ACCESS_SECRET>' > accessfile\n" +
      [
        "umask 077 && printf '%s' '" + token + "' > kfhost.token",
        'curl -fsSL ' + raw + '/' + commit + '/scripts/kfhost/bootstrap.py -o kfhost-bootstrap.py',
        'python3 kfhost-bootstrap.py --repo ' +
          repo +
          ' --commit ' +
          commit +
          ' --worker-url ' +
          origin +
          prefix +
          ' --box-id ' +
          id +
          ' --token-file kfhost.token --access-id ' +
          accessId +
          ' --access-secret-file accessfile',
      ].join(' && ');
    return json({ box_id: id, shown_once: true, command });
  }
  async adminCmd(id, body) {
    const config = await this.get('config');
    if (!config || config.box_id !== id) throw new HostError('NOT_FOUND', 404);
    const cmd = validateCommand(config.profile, body);
    const now = this.now();
    // Owned is not enough: the admin must have opened a window (commands_until, at most 4 h).
    if (!commandsOpen(config, now)) throw new HostError('CMD_DISABLED', 403);
    const sockets = this.live(now);
    if (!sockets.length) throw new HostError('NOT_CONNECTED', 409);
    const cmd_id = randomHex(8);
    const commands = (await this.get('commands')) ?? [];
    commands.push({
      cmd_id,
      at: now,
      ...('argv' in cmd
        ? { argv: cmd.argv.map((a) => a.slice(0, 256)).slice(0, 16) }
        : { shell: cmd.shell.slice(0, 512) }),
      output: '',
      truncated: false,
      rc: null,
      done: false,
    });
    await this.put('commands', commands.slice(-MAX_COMMANDS));
    // The daemon verifies freshness against commands_until_ms before it runs anything.
    this.send(sockets[0], {
      type: 'cmd',
      cmd_id,
      ...cmd,
      commands_until_ms: config.commands_until,
    });
    return json({ cmd_id });
  }

  // ---- offers (called by the planner, internal origin) -----------------------------------------
  async makeOffer(body) {
    const now = this.now();
    const snap = await this.snapshot();
    if (!snap) throw new HostError('NOT_FOUND', 404);
    if (!snap.config.enabled) throw new HostError('HOST_DISABLED', 409);
    if (!snap.connected) throw new HostError('NOT_CONNECTED', 409);
    if (!isObject(body) || !/^[A-Za-z0-9_.:-]{1,64}$/.test(body.request_id ?? ''))
      throw new HostError('OFFER_REQUEST', 400);
    // A result of this run counts only for a tier the box is enabled for: refuse the offer otherwise.
    const tier = normalTier(body.tier) === 'largest' ? 'large' : normalTier(body.tier);
    if (!tier || tier === 'adhoc') throw new HostError('OFFER_TIER', 400);
    if (!snap.config.tiers.includes(tier)) throw new HostError('TIER_NOT_ENABLED', 409);
    // Idempotent by request_id: the same request again after an unacked offer (expired, lost or
    // still waiting) is a REDELIVERY with the same offer_id and a fresh run token; once acked it
    // is answered without sending anything.
    const previous = await this.get('offer');
    const again =
      previous &&
      body.request_id === previous.request_id &&
      now < previous.deadline_ms &&
      ['offered', 'lost', 'acked', 'ready'].includes(previous.status)
        ? previous
        : null;
    if (['acked', 'ready'].includes(again?.status))
      return json({ offer_id: again.offer_id, run_id: again.run_id, duplicate: true });
    if (!again && (snap.state !== 'idle' || snap.offer_pending)) throw new HostError('BUSY', 409);
    const run = body.run;
    if (
      !isObject(run) ||
      !numericId(run.run) ||
      !numericId(run.attempt) ||
      !/^[0-9a-f]{32}$/.test(run.nonce ?? '') ||
      // The daemon returns nonce, runner_nonce and sha in its result (the existing result schema).
      !/^[0-9a-f]{64}$/.test(run.runner_nonce ?? '') ||
      !Number.isSafeInteger(run.reportExpiry) ||
      !Number.isSafeInteger(body.deadline_ms) ||
      body.deadline_ms <= now ||
      body.deadline_ms > now + 24 * 3600 * 1000
    )
      throw new HostError('OFFER_RUN', 400);
    const assignment = validateAssignment(
      snap.config.profile,
      body.assignment,
      this.repoAllowlist(),
    );
    const artifacts = validateKnownArtifacts(body.artifacts);
    // A consumer may be handed a seeder's artifact: the Worker builds the reference from the
    // seeder's own record (never from the planner or the box), and a borrowed consumer never gets one.
    if (body.seed_from !== undefined) {
      if (!seedAllowed(snap.config.profile)) throw new HostError('SEED_NOT_ALLOWED', 403);
      if (!BOX_ID.test(body.seed_from ?? '') || body.seed_from === snap.config.box_id)
        throw new HostError('OFFER_SEED', 400);
      const r = await this.stub(body.seed_from).fetch(
        'https://internal.invalid/internal/seed?consumer=' + snap.config.profile,
      );
      if (r.status === 403) throw new HostError('SEED_NOT_ALLOWED', 403);
      const rec = r.ok ? await r.json() : null;
      if (!rec || typeof rec.name !== 'string') throw new HostError('SEED_UNAVAILABLE', 409);
      assignment.seed = validateSeedRef({
        url: rec.url,
        fallback_url: rec.fallback_url ?? null,
        sha256: rec.sha256,
        size: rec.size,
        name: rec.name,
      });
    }
    const key = run.run + '-' + run.attempt;
    let run_token;
    try {
      run_token = await new RunTokens(this.env.RUN_TOKEN_SECRET).issue({
        run: run.run,
        attempt: run.attempt,
        sha: assignment.commit,
        nonce: run.nonce,
        key,
        label: 'kf-ci-' + key,
        reportExpiry: run.reportExpiry,
        // Results are bound to this box and profile (customHostResultCountable).
        box: snap.config.box_id,
        hp: snap.config.profile,
      });
    } catch {
      // The run fields were validated above, so a refusal here is the Worker's own secret.
      throw new HostError('TOKEN_UNAVAILABLE', 503);
    }
    const sockets = this.live(now);
    if (!sockets.length) throw new HostError('NOT_CONNECTED', 409);
    const offer_id = again ? again.offer_id : randomHex(8);
    // The run token goes to the box only; it is never stored here. The record keeps what ties a
    // result to this box (box, profile, tier, nonce) and the generation a seed must match.
    await this.put('offer', {
      offer_id,
      request_id: body.request_id,
      run_id: key,
      status: 'offered',
      offeredAt: now,
      deadline_ms: body.deadline_ms,
      generation: again ? again.generation : (previous?.generation ?? 0) + 1,
      deliveries: again ? (again.deliveries ?? 1) + 1 : 1,
      box_id: snap.config.box_id,
      profile: snap.config.profile,
      tier,
      nonce: run.nonce,
      ...(artifacts ? { artifacts } : {}),
      ...(again?.probe ? { probe: again.probe } : {}),
    });
    if (!again) await this.ctx.storage.delete('seed');
    this.send(sockets[0], {
      type: 'offer',
      offer_id,
      request_id: body.request_id,
      run_id: key,
      run_token,
      // What the result schema needs back: nonce, runner_nonce and sha (the commit).
      run: { nonce: run.nonce, runner_nonce: run.runner_nonce, sha: assignment.commit },
      assignment,
      deadline_ms: body.deadline_ms,
    });
    return json({ offer_id, run_id: key });
  }
  async cancelOffer(body) {
    const offer = await this.get('offer');
    if (!offer || !isObject(body) || body.offer_id !== offer.offer_id)
      throw new HostError('UNKNOWN_OFFER', 404);
    if (offer.status === 'offered' || offer.status === 'acked') {
      for (const w of this.live(this.now()))
        this.send(w, { type: 'cancel', offer_id: offer.offer_id });
      await this.put('offer', { ...offer, status: 'cancelled' });
    }
    return json({ ok: true });
  }
}

// Planner helper: snapshots of every enrolled box, for hostsFor() in hosts.js.
export async function hostSnapshots(env) {
  const r = await env.HOSTS.get(env.HOSTS.idFromName(INDEX_NAME)).fetch(
    'https://internal.invalid/internal/snapshots',
  );
  return r.ok ? (await r.json()).snapshots : [];
}
// Planner helper: offer a run to a chosen box.
export async function offerToHost(env, boxId, offer) {
  return env.HOSTS.get(env.HOSTS.idFromName(boxId)).fetch(
    'https://internal.invalid/internal/offer',
    {
      method: 'POST',
      body: JSON.stringify(offer),
    },
  );
}
