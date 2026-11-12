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
  SEED_NAME,
  boxTokenMatches,
  hashBoxToken,
  mintBoxToken,
  probeSeeder,
  sanitizeCapabilities,
  seedFallbackUrl,
  seedUrl,
  validateAssignment,
  validateCommand,
  validateHostConfig,
} from './hosts.js';
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
const SEED_PATH = /^\/([A-Za-z0-9_-]{16,128})\/([A-Za-z0-9._-]{1,100})$/;

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
  live(now) {
    // A socket that completed hello and was heard from within LIVENESS_MS.
    return this.openSockets().filter((w) => w.deserializeAttachment()?.helloed);
  }

  // ---- views -------------------------------------------------------------------------------
  async state() {
    const [config, caps, hb, offer, version, tokenHash, tokenAt, seed, commands] =
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
        ].map((k) => this.get(k)),
      );
    return { config, caps, hb, offer, version, tokenHash, tokenAt, seed, commands };
  }
  offerPending(offer, now) {
    if (!offer || now >= offer.deadline) return false;
    if (offer.status === 'offered') return now - offer.offeredAt < ACK_TIMEOUT_MS;
    return offer.status === 'acked';
  }
  async snapshot() {
    const s = await this.state(),
      now = this.now();
    if (!s.config) return null;
    const connected =
      this.live(now).length > 0 && s.hb !== undefined && now - s.hb.ts < LIVENESS_MS;
    return {
      box_id: s.config.box_id,
      config: s.config,
      capabilities: s.caps ?? {},
      connected,
      state: connected ? s.hb.state : 'idle',
      run_id: connected ? (s.hb.run_id ?? null) : null,
      last_heartbeat: s.hb?.ts ?? 0,
      offer_pending: this.offerPending(s.offer, now),
    };
  }
  async view() {
    const s = await this.state(),
      snap = await this.snapshot();
    if (!snap) return null;
    const offer = s.offer ? { ...s.offer } : null;
    return {
      ...snap,
      version: s.version ?? null,
      token_set: typeof s.tokenHash === 'string',
      token_minted_at: s.tokenAt ?? null,
      offer,
      seed: s.seed ?? null,
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
    if ((m = /^\/admin\/hosts\/([^/]+)(\/token|\/cmd)?$/.exec(path)) && BOX_ID.test(m[1])) {
      const id = m[1];
      if (!m[2] && method === 'GET') {
        const v = await this.view();
        if (!v || v.box_id !== id) throw new HostError('NOT_FOUND', 404);
        return json(v);
      }
      if (!m[2] && method === 'PUT') return this.adminPut(id, await readJson(request));
      if (!m[2] && method === 'DELETE') return this.adminDelete(id);
      if (m[2] === '/token' && method === 'POST') return this.adminToken(id);
      if (m[2] === '/cmd' && method === 'POST') return this.adminCmd(id, await readJson(request));
    }
    if (path === '/internal/snapshot' && method === 'GET') {
      const snap = await this.snapshot();
      if (!snap) throw new HostError('NOT_FOUND', 404);
      return json(snap);
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
    const config = await this.get('config');
    const bearer = /^Bearer (.+)$/.exec(request.headers.get('Authorization') ?? '')?.[1];
    // Unknown box, no token minted and a wrong token are indistinguishable (4401).
    const authed =
      !!config &&
      config.box_id === id &&
      (await boxTokenMatches(await this.get('tokenHash'), bearer));
    const reject = (code, reason) => {
      // A plain (non-hibernating) accept: the socket is closed at once, and workerd/miniflare
      // deliver an immediate close to the client only for such a socket.
      server.accept();
      server.close(code, reason);
      return new Response(null, { status: 101, webSocket: client });
    };
    if (!authed) return reject(4401, 'bad token');
    if (!config.enabled) return reject(4403, 'box disabled');
    // Replace-on-reconnect: the newest authenticated connection wins.
    for (const old of this.ctx.getWebSockets()) {
      try {
        old.close(4409, 'replaced');
      } catch {}
    }
    this.ctx.acceptWebSocket(server);
    server.serializeAttachment({ helloed: false });
    // The box must say hello within the liveness window or it is dropped.
    await this.ctx.storage.setAlarm(this.now() + LIVENESS_MS);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, message) {
    if (ws.readyState !== OPEN) return;
    if (typeof message !== 'string' || message.length > MAX_FRAME) return ws.close(1009, 'frame');
    let m;
    try {
      m = JSON.parse(message);
    } catch {
      return ws.close(1007, 'json');
    }
    if (!isObject(m) || (m.v !== undefined && m.v !== 1) || typeof m.type !== 'string')
      return ws.close(1008, 'frame');
    const attachment = ws.deserializeAttachment() ?? {};
    const config = await this.get('config');
    if (!config?.enabled) return ws.close(4403, 'box disabled');
    if (m.type === 'hello') {
      if (attachment.helloed || m.box_id !== config.box_id) return ws.close(1008, 'hello');
      const now = this.now();
      await this.put('caps', sanitizeCapabilities(m.capabilities));
      await this.put('version', typeof m.version === 'string' ? m.version.slice(0, 64) : null);
      await this.put('hb', { ts: now, state: 'idle' });
      ws.serializeAttachment({ helloed: true });
      this.send(ws, {
        type: 'welcome',
        box_id: config.box_id,
        profile: config.profile,
        heartbeat_s: HEARTBEAT_S,
      });
      await this.ctx.storage.setAlarm(now + LIVENESS_MS);
      return;
    }
    if (!attachment.helloed) return ws.close(1008, 'hello first');
    switch (m.type) {
      case 'heartbeat':
        return this.onHeartbeat(m);
      case 'ack':
      case 'nack':
        return this.onAck(ws, m);
      case 'seed_register':
        return this.onSeed(ws, m, config);
      case 'cmd_output':
      case 'cmd_done':
        if (config.profile !== 'owned') return ws.close(1008, 'commands are for owned boxes');
        return this.onCommand(m);
      default:
        return this.send(ws, { type: 'error', code: 'UNKNOWN_TYPE' });
    }
  }
  async onHeartbeat(m) {
    const now = this.now();
    if (m.state !== 'idle' && m.state !== 'busy') return;
    const hb = { ts: now, state: m.state };
    if (typeof m.run_id === 'string' && /^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(m.run_id))
      hb.run_id = m.run_id;
    await this.put('hb', hb);
    const offer = await this.get('offer');
    if (
      offer?.status === 'acked' &&
      hb.state === 'idle' &&
      !hb.run_id &&
      now - offer.ackedAt > ACK_GRACE_MS
    )
      await this.put('offer', { ...offer, status: 'done' });
    await this.ctx.storage.setAlarm(now + LIVENESS_MS);
  }
  async onAck(ws, m) {
    const offer = await this.get('offer');
    // An ack that arrives after the ack timeout (or after the box was marked lost) still counts
    // while the offer's deadline has not passed: the daemon acks only after it has fetched and
    // verified the commit, and a dropped connection must not lose that ack.
    if (
      !offer ||
      m.offer_id !== offer.offer_id ||
      !['offered', 'lost'].includes(offer.status) ||
      this.now() >= offer.deadline
    )
      return this.send(ws, { type: 'error', code: 'UNKNOWN_OFFER' });
    if (m.type === 'ack')
      await this.put('offer', { ...offer, status: 'acked', ackedAt: this.now() });
    else
      await this.put('offer', {
        ...offer,
        status: 'nacked',
        reason: typeof m.reason === 'string' ? m.reason.slice(0, 128) : '',
      });
  }
  async onSeed(ws, m, config) {
    const answer = (ok, reason) =>
      this.send(ws, { type: 'seed_ack', ok, ...(ok ? {} : { reason }) });
    const offer = await this.get('offer');
    const a = m.artifact;
    const sp = SEED_PATH.exec(typeof m.path === 'string' ? m.path : '');
    if (
      !offer ||
      offer.status !== 'acked' ||
      (m.offer_id !== undefined && m.offer_id !== offer.offer_id) ||
      (m.run_id !== undefined && m.run_id !== offer.run_id) ||
      !isObject(a) ||
      !SEED_NAME.test(a.name ?? '') ||
      !/^[0-9a-f]{64}$/.test(a.sha256 ?? '') ||
      !Number.isSafeInteger(a.size) ||
      a.size < 1 ||
      !sp ||
      sp[2] !== a.name
    )
      return answer(false, 'BAD_REGISTRATION');
    if (!config.seeder?.enabled) return answer(false, 'SEEDER_DISABLED');
    // Only the admin-configured address is contacted; nothing the box said selects it.
    const probe = await probeSeeder({
      fetcher: this.fetcher,
      ip: config.seeder.remote_ip,
      port: config.seeder.external_port,
      token: sp[1],
    });
    if (!probe.ok) return answer(false, probe.reason);
    await this.put('seed', {
      offer_id: offer.offer_id,
      // All four fields are the Worker's: the box's words only selected a path that was probed.
      url: seedUrl(config.seeder.remote_ip, config.seeder.external_port, sp[1], a.name),
      fallback_url: seedFallbackUrl(
        config.seeder.remote_ip,
        config.seeder.external_port,
        sp[1],
        a.name,
        probe.hostname,
      ),
      sha256: a.sha256,
      size: a.size,
      registered_at: this.now(),
    });
    return answer(true);
  }
  async onCommand(m) {
    const commands = (await this.get('commands')) ?? [];
    const c = commands.find((x) => x.cmd_id === m.cmd_id);
    if (!c) return;
    if (m.type === 'cmd_done') {
      if (Number.isInteger(m.rc)) c.rc = m.rc;
      c.done = true;
    } else if (typeof m.data === 'string' && (m.stream === 'stdout' || m.stream === 'stderr')) {
      const room = MAX_CMD_OUTPUT - c.output.length;
      c.output += m.data.slice(0, Math.max(0, room));
      if (m.data.length > room) c.truncated = true;
      if (m.eof === true) c.done = c.done ?? false;
    }
    await this.put('commands', commands);
  }
  async webSocketClose(ws, code, reason) {
    try {
      ws.close(code >= 1000 && code < 5000 && code !== 1005 && code !== 1006 ? code : 1000, reason);
    } catch {}
  }
  async webSocketError(ws) {
    try {
      ws.close(1011, 'error');
    } catch {}
  }
  // Liveness: no frame for LIVENESS_MS closes the socket.
  async alarm() {
    const now = this.now(),
      hb = await this.get('hb');
    const sockets = this.openSockets();
    if (!sockets.length) return;
    if (hb && now - hb.ts < LIVENESS_MS) {
      await this.ctx.storage.setAlarm(hb.ts + LIVENESS_MS);
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
    await this.put('config', { box_id: id, ...cfg, updated_at: this.now() });
    if (!cfg.enabled) this.closeAll(4403, 'box disabled');
    // A profile change takes effect on the next connection; do not keep a stale grant.
    else if (old && old.profile !== cfg.profile) this.closeAll(4409, 'profile changed');
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
  async adminToken(id) {
    const config = await this.get('config');
    if (!config || config.box_id !== id) throw new HostError('NOT_FOUND', 404);
    const token = mintBoxToken();
    await this.put('tokenHash', await hashBoxToken(token));
    await this.put('tokenAt', this.now());
    // Rotation: the old token's connections end now.
    this.closeAll(4401, 'token rotated');
    return json({ box_id: id, token, shown_once: true });
  }
  async adminCmd(id, body) {
    const config = await this.get('config');
    if (!config || config.box_id !== id) throw new HostError('NOT_FOUND', 404);
    const cmd = validateCommand(config.profile, body);
    const sockets = this.live(this.now());
    if (!sockets.length) throw new HostError('NOT_CONNECTED', 409);
    const cmd_id = randomHex(8);
    const commands = (await this.get('commands')) ?? [];
    commands.push({
      cmd_id,
      at: this.now(),
      ...('argv' in cmd
        ? { argv: cmd.argv.map((a) => a.slice(0, 256)).slice(0, 16) }
        : { shell: cmd.shell.slice(0, 512) }),
      output: '',
      truncated: false,
      rc: null,
      done: false,
    });
    await this.put('commands', commands.slice(-MAX_COMMANDS));
    this.send(sockets[0], { type: 'cmd', cmd_id, ...cmd });
    return json({ cmd_id });
  }

  // ---- offers (called by the planner, internal origin) -----------------------------------------
  async makeOffer(body) {
    const now = this.now();
    const snap = await this.snapshot();
    if (!snap) throw new HostError('NOT_FOUND', 404);
    if (!snap.config.enabled) throw new HostError('HOST_DISABLED', 409);
    if (!snap.connected) throw new HostError('NOT_CONNECTED', 409);
    // Idempotent by request_id: the same request again after an unacked offer (expired, lost or
    // still waiting) is a REDELIVERY with the same offer_id and a fresh run token; once acked it
    // is answered without sending anything.
    const previous = await this.get('offer');
    const again =
      previous &&
      isObject(body) &&
      body.request_id === previous.request_id &&
      now < previous.deadline &&
      ['offered', 'lost', 'acked'].includes(previous.status)
        ? previous
        : null;
    if (again?.status === 'acked')
      return json({ offer_id: again.offer_id, run_id: again.run_id, duplicate: true });
    if (!again && (snap.state !== 'idle' || snap.offer_pending)) throw new HostError('BUSY', 409);
    if (!isObject(body) || !/^[A-Za-z0-9_.:-]{1,64}$/.test(body.request_id ?? ''))
      throw new HostError('OFFER_REQUEST', 400);
    const run = body.run;
    if (
      !isObject(run) ||
      !numericId(run.run) ||
      !numericId(run.attempt) ||
      !/^[0-9a-f]{32}$/.test(run.nonce ?? '') ||
      !Number.isSafeInteger(run.reportExpiry) ||
      !Number.isSafeInteger(body.deadline) ||
      body.deadline <= now ||
      body.deadline > now + 24 * 3600 * 1000
    )
      throw new HostError('OFFER_RUN', 400);
    const assignment = validateAssignment(snap.config.profile, body.assignment);
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
      });
    } catch (e) {
      // The run fields were validated above, so a refusal here is the Worker's own secret.
      throw new HostError('TOKEN_UNAVAILABLE', 503);
    }
    const sockets = this.live(now);
    if (!sockets.length) throw new HostError('NOT_CONNECTED', 409);
    const offer_id = again ? again.offer_id : randomHex(8);
    // The run token goes to the box only; it is never stored here.
    await this.put('offer', {
      offer_id,
      request_id: body.request_id,
      run_id: key,
      status: 'offered',
      offeredAt: now,
      deadline: body.deadline,
      ...(again ? { deliveries: (again.deliveries ?? 1) + 1 } : { deliveries: 1 }),
    });
    if (!again) await this.ctx.storage.delete('seed');
    this.send(sockets[0], {
      type: 'offer',
      offer_id,
      request_id: body.request_id,
      run_id: key,
      run_token,
      assignment,
      deadline: body.deadline,
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
