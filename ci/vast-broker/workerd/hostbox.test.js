import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env } from '../test/helpers.js';
import { CANONICAL_REPO, SEED_PROBE_CONTENT, validateServerFrame } from '../src/hosts.js';

// FALSIFIER: in a real workerd the HostBox Durable Object accepts a bad or missing token, a
// disabled box or a stale connection; keeps two live connections; lets a non-owned box receive a
// command or a borrowed box receive more than a declarative offer; stores the token unhashed or
// the run token at all; or contacts any seeder address other than the configured one.
const bundle = await build({
  stdin: {
    contents: `
    import worker, {HostBox} from './src/index.js';
    export default worker;
    // Test seams only: a settable clock, a programmable seeder fetcher and a manual alarm.
    export class HostBoxFixture extends HostBox {
      constructor(ctx, env) {
        super(ctx, env);
        this.skew = 0;
        this.logs = [];
        for (const k of ['log', 'info', 'warn', 'error', 'debug'])
          console[k] = (...a) => this.logs.push(a.map(String).join(' '));
        this.probes = [];
        this.mode = 'ok';
        this.fetcher = async (url) => {
          this.probes.push(String(url));
          if (this.mode === 'first-fails' && String(url).includes('sslip.io')) return new Response('no', {status: 522});
          if (this.mode === 'hold')
            return new Promise((resolve) => (this.release = () => resolve(new Response(${JSON.stringify(SEED_PROBE_CONTENT)}))));
          if (this.mode === 'wrong') return new Response('something else');
          if (this.mode === 'redirect')
            return new Response(null, {status: 302, headers: {Location: 'http://10.0.0.1/'}});
          return new Response(${JSON.stringify(SEED_PROBE_CONTENT)});
        };
      }
      now() { return Date.now() + this.skew; }
      async fetch(request) {
        const p = new URL(request.url).pathname;
        if (p === '/__t/skew') { this.skew = Number(await request.text()); return new Response('ok'); }
        if (p === '/__t/mode') { this.mode = await request.text(); return new Response('ok'); }
        if (p === '/__t/alarm') { await this.alarm(); return new Response('ok'); }
        if (p === '/__t/storage') return Response.json([...(await this.ctx.storage.list())]);
        if (p === '/__t/release') { this.release?.(); return new Response('ok'); }
        if (p === '/__t/sockets') return Response.json(this.ctx.getWebSockets().map((w) => w.readyState));
        if (p === '/__t/evict') { this.pending.clear(); this.buckets = new WeakMap(); this.probing.clear(); return new Response('ok'); }
        if (p === '/__t/logs') return Response.json(this.logs);
        if (p === '/__t/probes') return Response.json(this.probes);
        return super.fetch(request);
      }
    }`,
    resolveDir: new URL('../', import.meta.url).pathname,
  },
  bundle: true,
  write: false,
  format: 'esm',
  target: 'es2022',
});

const config = (patch = {}) => ({
  enabled: true,
  profile: 'owned',
  tiers: ['small', 'middle'],
  seeder: { enabled: true, remote_ip: '93.184.216.34', external_port: 40022, inner_port: 8080 },
  window: null,
  opportunistic: false,
  notes: '',
  ...patch,
});
const caps = {
  gpu_name: 'NVIDIA GeForce RTX 4070',
  compute_cap: '8.9',
  vram_gb: 12,
  kvm: true,
  display: true,
  can_swap_driver: true,
  bogus: 1,
};
const sha = 'a'.repeat(40);
const assignment = { repo: CANONICAL_REPO, commit: sha, tree_key: 'b'.repeat(64) };
const run = {
  run: '123',
  attempt: '1',
  nonce: 'c'.repeat(32),
  runner_nonce: 'e'.repeat(64),
  reportExpiry: Math.floor(Date.now() / 1000) + 3600,
};
const offerBody = (extra = {}) => ({
  request_id: 'req-1',
  run,
  tier: 'small',
  assignment,
  deadline_ms: Date.now() + 3600000,
  ...extra,
});

const snapIdle = (r) => r.data.idle_since;
const runtime = async (t, extraBindings = {}) => {
  const pathPrefix = extraBindings.BROKER_PATH_PREFIX ? '/' + extraBindings.BROKER_PATH_PREFIX : '';
  const mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: bundle.outputFiles[0].text,
      compatibilityDate: '2026-10-23',
      bindings: { ...env, RENTING_ENABLED: 'false', ...extraBindings },
      durableObjects: {
        HOSTS: { className: 'HostBoxFixture', useSQLite: true },
        LEDGER: { className: 'BrokerLedger', useSQLite: true },
      },
      outboundService: () => {
        throw Error('No outbound traffic allowed');
      },
    }),
  );
  t.after(() => mf.dispose());
  const ns = await mf.getDurableObjectNamespace('HOSTS');
  const stub = (id) => ns.get(ns.idFromName(id));
  const call = async (id, method, path, body, raw) => {
    const r = await stub(id).fetch('https://internal.invalid' + path, {
      method,
      ...(body === undefined ? {} : { body: raw ?? JSON.stringify(body) }),
    });
    const text = await r.text();
    let data;
    try {
      data = JSON.parse(text);
    } catch {
      data = text;
    }
    return { status: r.status, data, text };
  };
  const put = (id, patch) => call(id, 'PUT', '/admin/hosts/' + id, config(patch));
  const mint = async (id) =>
    (await call(id, 'POST', '/admin/hosts/' + id + '/token', {})).data.token;
  // A connected client: frames collected, close code awaited.
  const connect = async (id, token, headers = {}) => {
    const r = await mf.dispatchFetch(
      'https://broker.example' + pathPrefix + '/hosts/' + id + '/ws',
      {
        headers: {
          Upgrade: 'websocket',
          ...(token ? { Authorization: 'Bearer ' + token } : {}),
          ...headers,
        },
      },
    );
    const sock = r.webSocket;
    assert.ok(sock, 'upgrade status ' + r.status);
    sock.accept();
    const frames = [];
    const waiters = [];
    let closed;
    const closedP = new Promise((res) => (closed = res));
    sock.addEventListener('message', (e) => {
      frames.push(JSON.parse(e.data));
      waiters.splice(0).forEach((w) => w());
    });
    sock.addEventListener('close', (e) => closed(e.code));
    const next = async (type) => {
      for (let i = 0; i < 200; i++) {
        const at = frames.findIndex((f) => f.type === type);
        if (at >= 0) return frames.splice(at, 1)[0];
        await new Promise((res) => {
          waiters.push(res);
          setTimeout(res, 25);
        });
      }
      throw Error('no frame ' + type);
    };
    const send = (frame) => sock.send(JSON.stringify({ v: 1, ...frame }));
    const code = () => {
      return Promise.race([closedP, new Promise((res) => setTimeout(() => res('open'), 4000))]);
    };
    return { sock, frames, next, send, code, closedP };
  };
  const open = (id) =>
    call(id, 'PUT', '/admin/hosts/' + id + '/commands-until', {
      commands_until: Date.now() + 3600000,
    });
  const hello = async (id, token, extra = {}) => {
    const c = await connect(id, token);
    c.send({ type: 'hello', box_id: id, version: '0.1', capabilities: caps, ...extra });
    c.welcome = await c.next('welcome');
    return c;
  };
  const view = async (id) => (await call(id, 'GET', '/admin/hosts/' + id)).data;
  // An upgrade the gateway answers over plain HTTP (never reaches a Durable Object).
  const upgradeStatus = async (path, headers) => {
    const r = await mf.dispatchFetch('https://broker.example' + path, { headers });
    await r.text();
    return r.status;
  };
  const wait = async (fn, what = 'condition') => {
    for (let i = 0; i < 150; i++) {
      if (await fn()) return;
      await new Promise((r) => setTimeout(r, 20));
    }
    assert.fail('timeout waiting for ' + what);
  };
  return { mf, stub, call, put, mint, connect, hello, view, upgradeStatus, wait, open };
};

test('admin: put validates, token shown once and stored hashed, views hide secrets', async (t) => {
  const h = await runtime(t);
  assert.equal(
    (await h.call('rack-1', 'PUT', '/admin/hosts/rack-1', { ...config(), profile: 'root' })).data
      .error,
    'CONFIG_PROFILE',
  );
  assert.equal((await h.call('rack-1', 'GET', '/admin/hosts/rack-1')).status, 404);
  const put = await h.put('rack-1');
  assert.equal(put.status, 200);
  assert.equal(put.data.token_set, false);
  const minted = await h.call('rack-1', 'POST', '/admin/hosts/rack-1/token', {});
  assert.match(minted.data.token, /^kfh_/);
  assert.equal(minted.data.shown_once, true);
  const v = await h.call('rack-1', 'GET', '/admin/hosts/rack-1');
  assert.equal(v.data.token_set, true);
  assert.ok(!v.text.includes(minted.data.token));
  // The raw storage holds a hash, never the token.
  const raw = await h.call('rack-1', 'GET', '/__t/storage');
  assert.ok(!raw.text.includes(minted.data.token));
  assert.match(Object.fromEntries(raw.data).tokenHash, /^[0-9a-f]{64}$/);
  const list = await h.call('__index__', 'GET', '/admin/hosts');
  assert.deepEqual(
    list.data.hosts.map((x) => x.box_id),
    ['rack-1'],
  );
  assert.ok(!list.text.includes(minted.data.token));
});

test('admin: only the owner-side internal origin reaches the DO', async (t) => {
  const h = await runtime(t);
  const r = await h.stub('rack-1').fetch('https://public.example/admin/hosts/rack-1');
  assert.equal(r.status, 404);
  await r.text();
  const g = await h.mf.dispatchFetch('https://broker.example/admin/hosts');
  assert.equal(g.status, 404);
  await g.text();
});

test('ws: unknown box, no token minted, wrong token and another box token all close 4401', async (t) => {
  const h = await runtime(t);
  const wrong = 'kfh_' + 'A'.repeat(43);
  assert.equal(await (await h.connect('ghost', wrong)).code(), 4401);
  await h.put('rack-1');
  assert.equal(await (await h.connect('rack-1', wrong)).code(), 4401, 'no token minted yet');
  const token = await h.mint('rack-1');
  assert.equal(await (await h.connect('rack-1', wrong)).code(), 4401);
  assert.equal(await (await h.connect('rack-1', token.slice(0, -1) + 'A')).code(), 4401);
  await h.put('rack-2');
  const other = await h.mint('rack-2');
  assert.equal(await (await h.connect('rack-1', other)).code(), 4401);
});

test('gateway pre-check: no token or a malformed one never reaches a Durable Object', async (t) => {
  const h = await runtime(t);
  const up = { Upgrade: 'websocket' };
  assert.equal(await h.upgradeStatus('/hosts/rack-1/ws', up), 401);
  assert.equal(
    await h.upgradeStatus('/hosts/rack-1/ws', { ...up, Authorization: 'Bearer garbage' }),
    401,
  );
  assert.equal(
    await h.upgradeStatus('/hosts/rack-1/ws', { ...up, Authorization: 'Basic abc' }),
    401,
  );
  // Nothing was created: the box list is still empty and no storage was touched.
  assert.deepEqual((await h.call('__index__', 'GET', '/admin/hosts')).data.hosts, []);
});

test('ws: a plain GET is refused and a bad box id is not routed', async (t) => {
  const h = await runtime(t);
  const r = await h.mf.dispatchFetch('https://broker.example/hosts/rack-1/ws');
  assert.equal(r.status, 401);
  await r.text();
  const b = await h.mf.dispatchFetch('https://broker.example/hosts/Rack_1/ws', {
    headers: { Upgrade: 'websocket' },
  });
  assert.equal(b.status, 404);
  await b.text();
});

test('ws: a disabled box closes 4403 only for a valid token', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1', { enabled: false });
  const token = await h.mint('rack-1');
  assert.equal(await (await h.connect('rack-1', token)).code(), 4403);
  assert.equal(await (await h.connect('rack-1', 'kfh_' + 'A'.repeat(43))).code(), 4401);
});

test('ws: hello stores sanitised capabilities, welcome names the server-side profile', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1', { profile: 'borrowed' });
  const token = await h.mint('rack-1');
  const c = await h.hello('rack-1', token, { capabilities: { ...caps, die: 'GB202' } });
  assert.deepEqual(c.welcome, {
    v: 1,
    type: 'welcome',
    box_id: 'rack-1',
    profile: 'borrowed',
    heartbeat_s: 15,
    commands_until_ms: null,
    cmd_output_per_s: 20,
  });
  const v = await h.view('rack-1');
  assert.equal(v.connected, true);
  assert.equal(v.state, 'idle');
  assert.equal('bogus' in v.capabilities, false);
  assert.equal(v.capabilities.compute_cap, '8.9');
  // The die is derived by the Worker from name and compute capability, never taken from the box.
  assert.equal(v.capabilities.die, 'AD104');
  assert.equal(v.version, '0.1');
  assert.equal(typeof snapIdle(await h.call('rack-1', 'GET', '/internal/snapshot')), 'number');
  const snaps = (await h.call('__index__', 'GET', '/internal/snapshots')).data.snapshots;
  assert.equal(snaps[0].connected, true);
});

test('ws: frames before hello, bad json, wrong version and a foreign box id close the socket', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const token = await h.mint('rack-1');
  const early = await h.connect('rack-1', token);
  early.send({ type: 'heartbeat', state: 'idle' });
  assert.equal(await early.code(), 1008);
  const junk = await h.connect('rack-1', token);
  junk.sock.send('not json');
  assert.equal(await junk.code(), 1007);
  const version = await h.connect('rack-1', token);
  version.sock.send(JSON.stringify({ v: 2, type: 'hello' }));
  assert.equal(await version.code(), 1008);
  const foreign = await h.connect('rack-1', token);
  foreign.send({ type: 'hello', box_id: 'rack-2', capabilities: caps });
  assert.equal(await foreign.code(), 1008);
  assert.equal((await h.view('rack-1')).connected, false);
});

test('liveness: a box silent for 45 s is closed 4408 and no longer connected', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  c.send({ type: 'heartbeat', state: 'idle' });
  await new Promise((r) => setTimeout(r, 200));
  await h.call('rack-1', 'POST', '/__t/skew', 1, '30000');
  await h.call('rack-1', 'POST', '/__t/alarm', 1, '');
  assert.equal((await h.view('rack-1')).connected, true, '30 s is still alive');
  await h.call('rack-1', 'POST', '/__t/skew', 1, '50000');
  assert.equal((await h.view('rack-1')).connected, false, 'snapshot already stale at 50 s');
  await h.call('rack-1', 'POST', '/__t/alarm', 1, '');
  assert.equal(await c.code(), 4408);
  assert.equal((await h.view('rack-1')).connected, false);
});

test('heartbeat keeps the box alive and reports busy with its run', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  c.send({ type: 'heartbeat', state: 'busy', run_id: '123-1' });
  for (let i = 0; i < 100; i++) {
    const v = await h.view('rack-1');
    if (v.state === 'busy') {
      assert.equal(v.run_id, '123-1');
      return;
    }
    await new Promise((r) => setTimeout(r, 20));
  }
  assert.fail('busy never seen');
});

test('a live connection is never kicked by a newer one; a silent one is replaced', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const token = await h.mint('rack-1');
  const first = await h.hello('rack-1', token);
  const second = await h.connect('rack-1', token);
  assert.equal(await second.code(), 4409, 'the NEW connection is refused');
  assert.equal((await h.view('rack-1')).connected, true, 'the live one is untouched');
  first.send({ type: 'heartbeat', state: 'idle' });
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal((await first.next('offer')).type, 'offer');
  // 46 s without a frame: the old socket is dead, the newer one takes over, the old closes 4409.
  await h.call('rack-1', 'POST', '/__t/skew', 1, '46000');
  const third = await h.hello('rack-1', token);
  assert.equal(await first.code(), 4409);
  assert.equal((await h.view('rack-1')).connected, true);
  assert.equal(third.welcome.type, 'welcome');
});

// Sockets the Durable Object itself still holds open (what a client sees can lag in miniflare: a
// close issued right after accept is not always delivered, so the server side is what is asserted).
const openSockets = async (h, id) =>
  (await h.call(id, 'GET', '/__t/sockets')).data.filter((state) => state === 1).length;

test('token rotation during an upgrade never leaves an old-token socket alive', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  for (let i = 0; i < 8; i++) {
    const old = await h.mint('rack-1');
    const [c] = await Promise.all([h.connect('rack-1', old), h.mint('rack-1')]);
    await h.wait(async () => (await openSockets(h, 'rack-1')) === 0, 'round ' + i + ' closed');
    assert.ok([4401, 'open'].includes(await c.code()), 'round ' + i);
    assert.equal((await h.view('rack-1')).connected, false, 'round ' + i);
  }
  // Also with a hello in flight right behind the upgrade.
  const old = await h.mint('rack-1');
  const c = await h.connect('rack-1', old);
  c.send({ type: 'hello', box_id: 'rack-1', capabilities: caps });
  await h.mint('rack-1');
  await h.wait(async () => (await openSockets(h, 'rack-1')) === 0, 'closed');
  assert.equal((await h.view('rack-1')).connected, false);
});

test('offer: carries a per-run token, is stored without it, ack and nack resolve it', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  const made = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal(made.status, 200);
  const offer = await c.next('offer');
  assert.equal(offer.offer_id, made.data.offer_id);
  assert.equal(offer.run_id, '123-1');
  assert.equal(offer.request_id, 'req-1');
  assert.deepEqual(offer.assignment, assignment);
  assert.match(offer.run_token, /^[\w-]+\.[\w-]+\.[\w-]+$/);
  const v = await h.view('rack-1');
  assert.equal(v.offer.status, 'offered');
  assert.ok(!JSON.stringify(v).includes(offer.run_token));
  const busy = await h.call(
    'rack-1',
    'POST',
    '/internal/offer',
    offerBody({ request_id: 'req-2' }),
  );
  assert.equal(busy.status, 409);
  assert.equal(busy.data.error, 'BUSY');
  c.send({ type: 'ack', offer_id: offer.offer_id });
  for (let i = 0; i < 100 && (await h.view('rack-1')).offer.status !== 'acked'; i++)
    await new Promise((r) => setTimeout(r, 20));
  assert.equal((await h.view('rack-1')).offer.status, 'acked');
  // A wrong offer id is answered with an error frame and changes nothing.
  c.send({ type: 'nack', offer_id: '0000000000000000' });
  assert.equal((await c.next('error')).code, 'UNKNOWN_OFFER');
  assert.equal((await h.view('rack-1')).offer.status, 'acked');
});

test('offer: nack frees the box; cancel is sent; unacked offers expire', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  let made = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const o1 = await c.next('offer');
  c.send({ type: 'nack', offer_id: o1.offer_id, reason: 'no disk' });
  for (let i = 0; i < 100 && (await h.view('rack-1')).offer.status !== 'nacked'; i++)
    await new Promise((r) => setTimeout(r, 20));
  assert.equal((await h.view('rack-1')).offer.reason, 'no disk');
  made = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal(made.status, 200);
  const o2 = await c.next('offer');
  const cancel = await h.call('rack-1', 'POST', '/internal/cancel', { offer_id: o2.offer_id });
  assert.equal(cancel.status, 200);
  assert.equal((await c.next('cancel')).offer_id, o2.offer_id);
  assert.equal((await h.call('rack-1', 'POST', '/internal/cancel', { offer_id: 'x' })).status, 404);
  // An offer nobody acks stops blocking the box after the ack timeout.
  made = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal(made.status, 200);
  await c.next('offer');
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody({ request_id: 'req-2' }))).status,
    409,
  );
  await h.call('rack-1', 'POST', '/__t/skew', 1, '31000');
  const snap = (await h.call('rack-1', 'GET', '/internal/snapshot')).data;
  assert.equal(snap.offer_pending, false);
});

test('offer refusals: not connected, disabled, bad run, bad deadline, bad request id', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const token = await h.mint('rack-1');
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody())).data.error,
    'NOT_CONNECTED',
  );
  await h.hello('rack-1', token);
  for (const [extra, code] of [
    [{ request_id: 'a b' }, 'OFFER_REQUEST'],
    [{ deadline_ms: 1 }, 'OFFER_RUN'],
    [{ run: { ...run, nonce: 'short' } }, 'OFFER_RUN'],
    [{ run: { ...run, run: '0' } }, 'OFFER_RUN'],
    [{ assignment: { ...assignment, commit: 'x' } }, 'OFFER_COMMIT'],
  ]) {
    const r = await h.call('rack-1', 'POST', '/internal/offer', offerBody(extra));
    assert.equal(r.data.error, code, code);
    assert.equal(r.status, 400);
  }
  assert.equal((await h.call('rack-1', 'GET', '/internal/snapshot')).data.offer_pending, false);
  assert.equal((await h.call('nobody', 'POST', '/internal/offer', offerBody())).status, 404);
});

test('borrowed: only repo, commit, tree key and allowlisted stage names are accepted', async (t) => {
  const h = await runtime(t);
  await h.put('b-1', { profile: 'borrowed' });
  const c = await h.hello('b-1', await h.mint('b-1'));
  for (const extra of [
    { env: { KF_A: '1' } },
    { requirements: {} },
    { script: 'x' },
    { stages: ['rm'] },
  ]) {
    const r = await h.call(
      'b-1',
      'POST',
      '/internal/offer',
      offerBody({ assignment: { ...assignment, ...extra } }),
    );
    assert.equal(r.status, 400, JSON.stringify(extra));
  }
  assert.equal(c.frames.filter((f) => f.type === 'offer').length, 0);
  const ok = await h.call(
    'b-1',
    'POST',
    '/internal/offer',
    offerBody({ assignment: { ...assignment, stages: ['windows_gate'] } }),
  );
  assert.equal(ok.status, 200);
  const offer = await c.next('offer');
  assert.deepEqual(Object.keys(offer.assignment).sort(), ['commit', 'repo', 'stages', 'tree_key']);
});

test('commands: owned only; container, standard and borrowed refuse, nothing is sent', async (t) => {
  const h = await runtime(t);
  for (const profile of ['container', 'standard', 'borrowed']) {
    const id = 'x-' + profile;
    await h.put(id, { profile });
    const c = await h.hello(id, await h.mint(id));
    assert.equal((await h.open(id)).status, 403, 'no command window for a non-owned box');
    for (const body of [{ argv: ['id'] }, { shell: 'id' }]) {
      const r = await h.call(id, 'POST', '/admin/hosts/' + id + '/cmd', body);
      assert.equal(r.status, 403);
      assert.equal(r.data.error, 'CMD_NOT_ALLOWED');
    }
    assert.equal(c.frames.filter((f) => f.type === 'cmd').length, 0);
    // A box that claims command output anyway is dropped.
    c.send({
      type: 'cmd_output',
      cmd_id: '0123456789abcdef',
      seq: 0,
      stream: 'stdout',
      data: 'hi',
    });
    assert.equal(await c.code(), 1008);
  }
});

test('commands: owned still needs an admin window (commands_until, at most 4 h)', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
  const c = await h.hello('own-1', await h.mint('own-1'));
  const cmd = (b = { shell: 'id' }) => h.call('own-1', 'POST', '/admin/hosts/own-1/cmd', b);
  assert.equal((await cmd()).data.error, 'CMD_DISABLED', 'unset by default');
  const until = (v) =>
    h.call('own-1', 'PUT', '/admin/hosts/own-1/commands-until', { commands_until: v });
  for (const bad of [Date.now() - 1000, Date.now() + 5 * 3600 * 1000, 'soon', 1.5])
    assert.equal((await until(bad)).data.error, 'COMMANDS_UNTIL', String(bad));
  assert.equal(
    (await h.call('own-1', 'PUT', '/admin/hosts/own-1/commands-until', { x: 1 })).status,
    400,
  );
  const set = await until(Date.now() + 60000);
  assert.equal(set.status, 200);
  assert.ok(set.data.config.commands_until > Date.now());
  assert.equal((await cmd()).status, 200);
  assert.equal((await c.next('cmd')).shell, 'id');
  // A config edit keeps the window; expiry closes it; clearing closes it.
  await h.put('own-1');
  assert.equal((await cmd()).status, 200);
  await h.call('own-1', 'POST', '/__t/skew', 1, '61000');
  assert.equal((await cmd()).data.error, 'CMD_DISABLED', 'expired');
  await h.call('own-1', 'POST', '/__t/skew', 1, '0');
  await until(Date.now() + 60000);
  assert.equal((await cmd()).status, 200);
  assert.equal((await until(null)).data.config.commands_until, null);
  assert.equal((await cmd()).data.error, 'CMD_DISABLED', 'cleared');
  // Changing the profile drops the window.
  await until(Date.now() + 60000);
  await h.put('own-1', { profile: 'standard' });
  await h.put('own-1', { profile: 'owned' });
  assert.equal((await h.view('own-1')).config.commands_until, null);
});

test('commands: owned box gets the frame, output and exit code are logged, bounded and batched', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
  await h.open('own-1');
  const c = await h.hello('own-1', await h.mint('own-1'));
  assert.equal(
    (await h.call('own-1', 'POST', '/admin/hosts/own-1/cmd', { argv: ['a'], shell: 'b' })).status,
    400,
  );
  const r = await h.call('own-1', 'POST', '/admin/hosts/own-1/cmd', { argv: ['nvidia-smi', '-L'] });
  assert.equal(r.status, 200);
  const cmd = await c.next('cmd');
  assert.deepEqual(cmd.argv, ['nvidia-smi', '-L']);
  assert.equal(cmd.cmd_id, r.data.cmd_id);
  const out = (seq, stream, data) =>
    c.send({ type: 'cmd_output', cmd_id: cmd.cmd_id, seq, stream, data });
  out(0, 'stdout', 'GPU 0\n');
  out(1, 'stdout', 'GPU 1\n');
  await new Promise((x) => setTimeout(x, 150));
  // Small frames are buffered, not written one by one: the raw log is still empty.
  const raw = Object.fromEntries((await h.call('own-1', 'GET', '/__t/storage')).data);
  assert.equal(raw.commands[0].output, '');
  assert.equal((await h.view('own-1')).commands[0].output, 'GPU 0\nGPU 1\n', 'a read flushes');
  for (let i = 0; i < 3; i++) out(2 + i, 'stderr', 'x'.repeat(16000));
  c.send({ type: 'cmd_done', cmd_id: cmd.cmd_id, rc: 0 });
  await h.wait(async () => (await h.view('own-1')).commands[0].done, 'done');
  const log = (await h.view('own-1')).commands[0];
  assert.equal(log.rc, 0);
  assert.ok(log.output.startsWith('GPU 0\n'));
  assert.ok(log.output.length <= 32768);
  assert.equal(log.truncated, true);
  // Output after done is ignored.
  const before = log.output;
  out(9, 'stdout', 'late');
  await new Promise((x) => setTimeout(x, 150));
  assert.equal((await h.view('own-1')).commands[0].output, before);
});

test('commands: refused while the box is not connected', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
  await h.open('own-1');
  const r = await h.call('own-1', 'POST', '/admin/hosts/own-1/cmd', { shell: 'id' });
  assert.equal(r.status, 409);
});

test('disabling a connected box closes it 4403; deleting wipes it', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const token = await h.mint('rack-1');
  const c = await h.hello('rack-1', token);
  await h.put('rack-1', { enabled: false });
  assert.equal(await c.code(), 4403);
  assert.equal(await (await h.connect('rack-1', token)).code(), 4403);
  await h.put('rack-1');
  const d = await h.call('rack-1', 'DELETE', '/admin/hosts/rack-1');
  assert.equal(d.status, 200);
  assert.equal((await h.view('rack-1')).error, 'NOT_FOUND');
  assert.equal(await (await h.connect('rack-1', token)).code(), 4401);
  assert.deepEqual((await h.call('__index__', 'GET', '/admin/hosts')).data.hosts, []);
});

test('token rotation closes the live connection 4401 and invalidates the old token', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const old = await h.mint('rack-1');
  const c = await h.hello('rack-1', old);
  const fresh = await h.mint('rack-1');
  assert.notEqual(fresh, old);
  assert.equal(await c.code(), 4401);
  assert.equal(await (await h.connect('rack-1', old)).code(), 4401);
  assert.equal((await h.hello('rack-1', fresh)).welcome.type, 'welcome');
});

// An acked offer on a connected box, ready to register a seed.
const ackedOffer = async (h, id, body = offerBody()) => {
  const c = await h.hello(id, await h.mint(id));
  const made = await h.call(id, 'POST', '/internal/offer', body);
  assert.equal(made.status, 200, made.text);
  const offer = await c.next('offer');
  c.send({ type: 'ack', offer_id: offer.offer_id });
  await h.wait(async () => (await h.view(id)).offer.status === 'acked', 'acked');
  return { c, offer };
};
const TOK = 'sd7k2m9q';
const ART = { name: 'kf3.tar', sha256: 'd'.repeat(64), size: 1234 };
const reg = (c, offer, extra = {}) =>
  c.send({
    type: 'seed_register',
    offer_id: offer.offer_id,
    artifact: ART,
    path: '/' + TOK + '/kf3.tar',
    ...extra,
  });

test('seeder: probes only the configured address; the box cannot choose another', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const offer = await c.next('offer');
  // Not acked yet: refused.
  reg(c, offer);
  assert.deepEqual(await c.next('seed_ack'), {
    v: 1,
    type: 'seed_ack',
    ok: false,
    reason: 'BAD_REGISTRATION',
  });
  c.send({ type: 'ack', offer_id: offer.offer_id });
  await h.wait(async () => (await h.view('rack-1')).offer.status === 'acked', 'acked');
  // Extra address fields in the frame are not even a valid frame: nothing is probed.
  reg(c, offer, { ip: '198.51.100.1', port: 1, url: 'http://198.51.100.1:1/x' });
  assert.equal((await c.next('error')).code, 'BAD_FRAME');
  assert.deepEqual((await h.call('rack-1', 'GET', '/__t/probes')).data, []);
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).ok, true);
  assert.deepEqual((await h.call('rack-1', 'GET', '/__t/probes')).data, [
    'http://93-184-216-34.sslip.io:40022/' + TOK + '/probe.txt',
  ]);
  const v = await h.view('rack-1');
  assert.equal(v.seed.url, 'http://93.184.216.34:40022/' + TOK + '/kf3.tar');
  assert.equal(v.seed.fallback_url, 'http://93-184-216-34.sslip.io:40022/' + TOK + '/kf3.tar');
  assert.equal(v.seed.sha256, 'd'.repeat(64));
  assert.equal(v.seed.size, 1234);
  assert.equal(v.seed.source, 'seeder_reported');
  assert.equal(v.seed.seeder_profile, 'owned');
  // Invalid shapes (path name differs, bad token, traversal) are refused as frames.
  for (const path of [
    '/' + TOK + '/other.tar',
    '/AB12CD34/kf3.tar',
    '/ab12cd34567/kf3.tar',
    '/../x/kf3.tar',
  ]) {
    reg(c, offer, { path });
    assert.equal((await c.next('error')).code, 'BAD_FRAME', path);
  }
});

test('seeder: at most ONE probe per offer, its result is cached', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const { c, offer } = await ackedOffer(h, 'rack-1');
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'wrong');
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).reason, 'WRONG_CONTENT');
  // The network now looks fine, but the offer's probe is spent: the cached failure is answered.
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'ok');
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).reason, 'WRONG_CONTENT');
  reg(c, offer, { path: '/zz9y8x7w/kf3.tar' });
  assert.equal((await c.next('seed_ack')).reason, 'PROBE_USED');
  const probes = (await h.call('rack-1', 'GET', '/__t/probes')).data;
  assert.equal(probes.length, 2, 'two names of ONE probe (sslip.io, traefik.me), not more');
  assert.equal((await h.view('rack-1')).seed, null);
});

test('seeder: a redirect fails the probe', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const { c, offer } = await ackedOffer(h, 'rack-1');
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'redirect');
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).reason, 'REDIRECT');
});

test('seeder: a registration is checked against an independently known hash and size', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const known = { 'kf3.tar': { sha256: 'd'.repeat(64), size: 1234 } };
  const { c, offer } = await ackedOffer(h, 'rack-1', offerBody({ artifacts: known }));
  reg(c, offer, { artifact: { ...ART, sha256: 'e'.repeat(64) } });
  assert.equal((await c.next('seed_ack')).reason, 'HASH_MISMATCH');
  reg(c, offer, { artifact: { ...ART, size: 1 } });
  assert.equal((await c.next('seed_ack')).reason, 'HASH_MISMATCH');
  reg(c, offer, { artifact: { ...ART, name: 'evil.tar' }, path: '/' + TOK + '/evil.tar' });
  assert.equal((await c.next('seed_ack')).reason, 'ARTIFACT_UNKNOWN');
  assert.deepEqual(
    (await h.call('rack-1', 'GET', '/__t/probes')).data,
    [],
    'no probe for a refused registration',
  );
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).ok, true);
  assert.equal((await h.view('rack-1')).seed.source, 'worker_pinned');
});

test('seeds: handed to every consumer except a borrowed one (internal seed route)', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1', { profile: 'container' });
  const { c, offer } = await ackedOffer(h, 'rack-1');
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).ok, true);
  for (const consumer of ['owned', 'container', 'standard']) {
    const r = await h.call('rack-1', 'GET', '/internal/seed?consumer=' + consumer);
    assert.equal(r.status, 200, consumer);
    assert.equal(r.data.seeder_profile, 'container');
  }
  const r = await h.call('rack-1', 'GET', '/internal/seed?consumer=borrowed');
  assert.equal(r.status, 403);
  assert.equal(r.data.error, 'SEED_NOT_ALLOWED');
});

test('seeder: an offer replaced while the probe runs never writes a seed', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const { c, offer } = await ackedOffer(h, 'rack-1');
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'hold');
  reg(c, offer);
  await h.wait(
    async () => (await h.call('rack-1', 'GET', '/__t/probes')).data.length > 0,
    'probe started',
  );
  // Meanwhile the offer is cancelled and a NEW one (new generation) is made.
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/cancel', { offer_id: offer.offer_id })).status,
    200,
  );
  const again = await h.call(
    'rack-1',
    'POST',
    '/internal/offer',
    offerBody({ request_id: 'req-2' }),
  );
  assert.equal(again.status, 200);
  assert.notEqual(again.data.offer_id, offer.offer_id);
  await h.call('rack-1', 'POST', '/__t/release', 1, '');
  assert.equal((await c.next('seed_ack')).reason, 'STALE_OFFER');
  const v = await h.view('rack-1');
  assert.equal(v.seed, null);
  assert.equal(v.offer.offer_id, again.data.offer_id);
  assert.equal(v.offer.generation, 2);
  assert.equal(v.offer.probe, undefined, 'the stale probe result is not cached on the new offer');
});

test('seeder disabled in the admin config: refused without any fetch', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1', { seeder: { enabled: false } });
  const { c, offer } = await ackedOffer(h, 'rack-1');
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).reason, 'SEEDER_DISABLED');
  assert.deepEqual((await h.call('rack-1', 'GET', '/__t/probes')).data, []);
});

test('unknown frame types get an error frame, not a crash', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  c.send({ type: 'shell', cmd: 'id' });
  assert.equal((await c.next('error')).code, 'UNKNOWN_TYPE');
});

test('frames without v are accepted (the daemon omits it); another v is refused', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.connect('rack-1', await h.mint('rack-1'));
  c.sock.send(JSON.stringify({ type: 'hello', box_id: 'rack-1', capabilities: caps }));
  assert.equal((await c.next('welcome')).profile, 'owned');
});

test('redelivery: the same request after an unacked offer reuses the offer_id; a late ack counts', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  const first = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const o1 = await c.next('offer');
  // The ack is lost: the offer times out and the box looks free again.
  await h.call('rack-1', 'POST', '/__t/skew', 1, '31000');
  c.send({ type: 'heartbeat', state: 'idle' });
  await new Promise((r) => setTimeout(r, 100));
  assert.equal((await h.call('rack-1', 'GET', '/internal/snapshot')).data.offer_pending, false);
  const again = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal(again.status, 200);
  assert.equal(again.data.offer_id, first.data.offer_id, 'idempotent by offer_id');
  const o2 = await c.next('offer');
  assert.equal(o2.offer_id, o1.offer_id);
  assert.notEqual(o2.run_token, undefined);
  assert.equal((await h.view('rack-1')).offer.deliveries, 2);
  c.send({ type: 'ack', offer_id: o2.offer_id });
  for (let i = 0; i < 100 && (await h.view('rack-1')).offer.status !== 'acked'; i++)
    await new Promise((r) => setTimeout(r, 20));
  assert.equal((await h.view('rack-1')).offer.status, 'acked');
  // After the ack the same request is answered, not delivered again.
  const dup = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal(dup.data.duplicate, true);
  assert.equal(dup.data.offer_id, first.data.offer_id);
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(c.frames.filter((f) => f.type === 'offer').length, 0);
  // A different request while it is acked is busy.
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody({ request_id: 'req-2' }))).status,
    409,
  );
});

test('a late ack for an offer marked lost still counts until its deadline', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const o = await c.next('offer');
  // Silence for 50 s: the alarm marks the offer lost and drops the socket; the box reconnects.
  await h.call('rack-1', 'POST', '/__t/skew', 1, '50000');
  await h.call('rack-1', 'POST', '/__t/alarm', 1, '');
  assert.equal(await c.code(), 4408);
  assert.equal((await h.view('rack-1')).offer.status, 'lost');
  const back = await h.hello('rack-1', await h.mint('rack-1'));
  back.send({ type: 'ack', offer_id: o.offer_id });
  for (let i = 0; i < 100 && (await h.view('rack-1')).offer.status !== 'acked'; i++)
    await new Promise((r) => setTimeout(r, 20));
  assert.equal((await h.view('rack-1')).offer.status, 'acked');
});

test('admin: a seeder hostname cannot be configured', async (t) => {
  const h = await runtime(t);
  const seeder = {
    enabled: true,
    remote_ip: '93.184.216.34',
    external_port: 40022,
    inner_port: 8080,
    hostname: 'seed.example.org',
  };
  assert.equal((await h.put('rack-1', { seeder })).data.error, 'CONFIG_SEEDER');
});

test('seeder: the fallback_url names the mapper that passed the probe', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'first-fails');
  const { c, offer } = await ackedOffer(h, 'rack-1');
  reg(c, offer);
  assert.equal((await c.next('seed_ack')).ok, true);
  const v = await h.view('rack-1');
  assert.equal(v.seed.fallback_url, 'http://93-184-216-34.traefik.me:40022/' + TOK + '/kf3.tar');
  assert.equal(v.seed.url, 'http://93.184.216.34:40022/' + TOK + '/kf3.tar');
  assert.deepEqual((await h.call('rack-1', 'GET', '/__t/probes')).data, [
    'http://93-184-216-34.sslip.io:40022/' + TOK + '/probe.txt',
    'http://93-184-216-34.traefik.me:40022/' + TOK + '/probe.txt',
  ]);
  // Both names failing is "cannot open a port" (a fresh offer gets a fresh probe).
  await h.call('rack-1', 'POST', '/internal/cancel', { offer_id: offer.offer_id });
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'wrong');
  const made = await h.call(
    'rack-1',
    'POST',
    '/internal/offer',
    offerBody({ request_id: 'req-2' }),
  );
  const o2 = await c.next('offer');
  c.send({ type: 'ack', offer_id: o2.offer_id });
  await h.wait(async () => (await h.view('rack-1')).offer.status === 'acked', 'acked');
  reg(c, o2);
  assert.equal((await c.next('seed_ack')).ok, false);
  assert.equal(made.status, 200);
});

const jwtClaims = (token) => JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString());

test('offer: tier must be enabled for the box; the run token and record bind box, profile and tier', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1', { profile: 'standard', tiers: ['small'] });
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody({ tier: 'middle' }))).data.error,
    'TIER_NOT_ENABLED',
  );
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody({ tier: 'large' }))).data.error,
    'TIER_NOT_ENABLED',
  );
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody({ tier: 'weird' }))).data.error,
    'OFFER_TIER',
  );
  const { tier, ...noTier } = offerBody();
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', noTier)).data.error,
    'OFFER_TIER',
  );
  assert.equal(c.frames.filter((f) => f.type === 'offer').length, 0);
  const ok = await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal(ok.status, 200);
  const offer = await c.next('offer');
  const claims = jwtClaims(offer.run_token);
  assert.equal(claims.box, 'rack-1');
  assert.equal(claims.hp, 'standard');
  assert.equal(claims.run, '123');
  assert.equal(offer.deadline_ms > Date.now(), true);
  assert.equal('deadline' in offer, false);
  const v = await h.view('rack-1');
  assert.deepEqual(
    [v.offer.box_id, v.offer.profile, v.offer.tier, v.offer.nonce],
    ['rack-1', 'standard', 'small', 'c'.repeat(32)],
  );
});

test('offers carry the canonical repo URL for every profile, never an arbitrary one', async (t) => {
  const h = await runtime(t);
  for (const profile of ['owned', 'container', 'standard', 'borrowed']) {
    const id = 'r-' + profile;
    await h.put(id, { profile });
    const c = await h.hello(id, await h.mint(id));
    for (const repo of ['https://evil.example/kayfabe', 'reindertpelsma/kayfabe'])
      assert.equal(
        (
          await h.call(
            id,
            'POST',
            '/internal/offer',
            offerBody({ assignment: { ...assignment, repo } }),
          )
        ).data.error,
        'OFFER_REPO',
        profile,
      );
    assert.equal((await h.call(id, 'POST', '/internal/offer', offerBody())).status, 200);
    assert.equal((await c.next('offer')).assignment.repo, CANONICAL_REPO);
  }
});

test('a per-socket rate limit closes a flooding box 1008', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  for (let i = 0; i < 80; i++) c.send({ type: 'heartbeat', state: 'idle' });
  assert.equal(await c.code(), 1008);
  // An oversized frame is refused too.
  const d = await h.hello('rack-1', await h.mint('rack-1'));
  d.sock.send('x'.repeat(70000));
  assert.equal(await d.code(), 1009);
});

test('heartbeats write storage only on a state change or once a minute', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  const stored = async () =>
    Object.fromEntries((await h.call('rack-1', 'GET', '/__t/storage')).data).hb;
  const first = await stored();
  for (let i = 0; i < 5; i++) c.send({ type: 'heartbeat', state: 'idle' });
  await new Promise((r) => setTimeout(r, 200));
  assert.equal((await stored()).ts, first.ts, 'no write for repeated idle heartbeats');
  assert.equal((await h.view('rack-1')).connected, true, 'liveness still tracked');
  c.send({ type: 'heartbeat', state: 'busy', run_id: '123-1' });
  await h.wait(async () => (await stored()).state === 'busy', 'busy stored');
  await h.call('rack-1', 'POST', '/__t/skew', 1, '61000');
  c.send({ type: 'heartbeat', state: 'busy', run_id: '123-1' });
  await h.wait(async () => (await stored()).ts > first.ts + 60000, 'periodic write');
});

test('admin list shows idle_since and the hourly price; a bad price is refused', async (t) => {
  const h = await runtime(t);
  assert.equal((await h.put('rack-1', { hourly_price_usd: 51 })).data.error, 'CONFIG_PRICE');
  assert.equal((await h.put('rack-1', { hourly_price_usd: -1 })).data.error, 'CONFIG_PRICE');
  assert.equal((await h.put('rack-1', { hourly_price_usd: 'cheap' })).data.error, 'CONFIG_PRICE');
  assert.equal((await h.put('rack-1', { hourly_price_usd: 0.35 })).status, 200);
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  const row = () => h.call('__index__', 'GET', '/admin/hosts').then((r) => r.data.hosts[0]);
  const idle = (await row()).idle_since;
  assert.equal(typeof idle, 'number');
  assert.equal((await row()).config.hourly_price_usd, 0.35);
  c.send({ type: 'heartbeat', state: 'busy', run_id: '123-1' });
  await h.wait(async () => (await row()).idle_since === null, 'busy hides idle_since');
  await new Promise((r) => setTimeout(r, 30));
  c.send({ type: 'heartbeat', state: 'idle' });
  await h.wait(async () => typeof (await row()).idle_since === 'number', 'idle again');
  assert.ok((await row()).idle_since >= idle);
});

const COMMIT = '0123456789abcdef0123456789abcdef01234567';
const enrol = (h, id) => h.call(id, 'POST', '/admin/hosts/' + id + '/enrol', {});

test('enrol: rotates the token, closes old sockets, returns the command once, no secret in logs', async (t) => {
  const prefix = 'P'.repeat(40);
  const h = await runtime(t, { HOST_DAEMON_COMMIT: COMMIT, BROKER_PATH_PREFIX: prefix });
  await h.put('rack-1');
  const old = await h.mint('rack-1');
  const c = await h.hello('rack-1', old);
  const r = await enrol(h, 'rack-1');
  assert.equal(r.status, 200);
  assert.equal(await c.code(), 4401, 'the old socket is closed');
  assert.equal(await (await h.connect('rack-1', old)).code(), 4401, 'the old token is dead');
  const { command } = r.data;
  assert.deepEqual(Object.keys(r.data).sort(), ['box_id', 'command', 'shown_once']);
  const token = /printf '%s' '(kfh_[A-Za-z0-9_-]{43})' > kfhost\.token/.exec(command)?.[1];
  assert.ok(token, 'the token is delivered as a file write by a shell builtin');
  assert.match(command, new RegExp('--worker-url https://broker\\.example/' + prefix + ' '));
  assert.match(command, /--box-id rack-1 /);
  assert.match(command, new RegExp('--commit ' + COMMIT));
  assert.match(
    command,
    new RegExp(
      'raw\\.githubusercontent\\.com/reindertpelsma/kayfabe/' +
        COMMIT +
        '/scripts/kfhost/bootstrap\\.py',
    ),
  );
  assert.ok(!/--token(?!-file)/.test(command), 'the token is never an argv value');
  assert.equal(command.split(token).length, 2, 'the token appears exactly once');
  // The new token works; the response is the only place it ever appears.
  assert.equal((await h.hello('rack-1', token)).welcome.type, 'welcome');
  const view = await h.call('rack-1', 'GET', '/admin/hosts/rack-1');
  const raw = await h.call('rack-1', 'GET', '/__t/storage');
  const logs = await h.call('rack-1', 'GET', '/__t/logs');
  for (const where of [
    view.text,
    raw.text,
    logs.text,
    JSON.stringify((await h.call('__index__', 'GET', '/admin/hosts')).data),
  ])
    assert.ok(!where.includes(token), 'token leaked');
  // Enrolling again rotates again: the earlier command is useless.
  const again = await enrol(h, 'rack-1');
  assert.notEqual(again.data.command, command);
  assert.equal(await (await h.connect('rack-1', token)).code(), 4401);
});

test('enrol refuses without a pinned HOST_DAEMON_COMMIT and for an unknown box, rotating nothing', async (t) => {
  for (const bad of [undefined, '', 'main', 'A'.repeat(40), 'a'.repeat(39)]) {
    const h = await runtime(t, bad === undefined ? {} : { HOST_DAEMON_COMMIT: bad });
    await h.put('rack-1');
    const token = await h.mint('rack-1');
    const r = await enrol(h, 'rack-1');
    assert.equal(r.status, 503, String(bad));
    assert.equal(r.data.error, 'ENROL_UNAVAILABLE');
    assert.equal(
      (await h.hello('rack-1', token)).welcome.type,
      'welcome',
      'the token was not rotated',
    );
  }
  const h = await runtime(t, { HOST_DAEMON_COMMIT: COMMIT });
  assert.equal((await enrol(h, 'nobody')).status, 404);
});

test('hello: enforced flags are stored read-only; idle heartbeats with idle_since keep the box alive', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const flags = { distrust_us: false, allow_cmd: true, container: false };
  const c = await h.hello('rack-1', await h.mint('rack-1'), { enforced: flags });
  assert.deepEqual((await h.view('rack-1')).enforced, flags);
  // The admin config has no way to edit them.
  assert.equal(
    (await h.put('rack-1', { enforced: { allow_cmd: false } })).data.error,
    'CONFIG_KEYS',
  );
  // An idle heartbeat as the daemon sends it is accepted (not a protocol error) and keeps liveness.
  c.send({ type: 'heartbeat', ts: 1800000000, state: 'idle', idle_since: 1799999000000 });
  await new Promise((r) => setTimeout(r, 100));
  await h.call('rack-1', 'POST', '/__t/skew', 1, '30000');
  c.send({ type: 'heartbeat', ts: 1800000030, state: 'idle', idle_since: 1799999000000 });
  await new Promise((r) => setTimeout(r, 100));
  await h.call('rack-1', 'POST', '/__t/skew', 1, '60000');
  c.send({ type: 'heartbeat', ts: 1800000060, state: 'idle', idle_since: 1799999000000 });
  await new Promise((r) => setTimeout(r, 100));
  await h.call('rack-1', 'POST', '/__t/alarm', 1, '');
  assert.equal((await h.view('rack-1')).connected, true);
  assert.equal(await openSockets(h, 'rack-1'), 1);
  // A bad value is answered with an error frame, not a close.
  c.send({ type: 'heartbeat', state: 'idle', idle_since: 'now' });
  assert.equal((await c.next('error')).code, 'BAD_FRAME');
});

test('ack, ready, nack: ack is accepted, ready is verified, nack after an ack fails the run', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const o = await c.next('offer');
  // ready before ack is refused; ack; ready.
  c.send({ type: 'ready', offer_id: o.offer_id });
  assert.equal((await c.next('error')).code, 'UNKNOWN_OFFER');
  c.send({ type: 'ack', offer_id: o.offer_id });
  await h.wait(async () => (await h.view('rack-1')).offer.status === 'acked', 'acked');
  assert.equal((await h.call('rack-1', 'GET', '/internal/snapshot')).data.offer_pending, true);
  c.send({ type: 'ready', offer_id: o.offer_id });
  await h.wait(async () => (await h.view('rack-1')).offer.status === 'ready', 'ready');
  assert.equal((await h.call('rack-1', 'GET', '/internal/snapshot')).data.offer_pending, true);
  // The same request after ready is a duplicate, not a second delivery.
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody())).data.duplicate,
    true,
  );
  // A nack after the ack (the daemon's verification failed): failed, released, unmet.
  c.send({ type: 'nack', offer_id: o.offer_id, reason: 'tree key mismatch' });
  await h.wait(async () => (await h.view('rack-1')).offer.status === 'failed', 'failed');
  const v = await h.view('rack-1');
  assert.equal(v.offer.reason, 'tree key mismatch');
  assert.equal(v.offer.unmet, true);
  assert.equal((await h.call('rack-1', 'GET', '/internal/snapshot')).data.offer_pending, false);
  // The box can take a new offer; a nack of the failed one is now unknown.
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody({ request_id: 'req-2' }))).status,
    200,
  );
  await c.next('offer');
  c.send({ type: 'nack', offer_id: o.offer_id });
  assert.equal((await c.next('error')).code, 'UNKNOWN_OFFER');
});

test('offer: the run section carries nonce, runner_nonce and the commit; runner_nonce is required', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  const { runner_nonce, ...noRunner } = run;
  assert.equal(
    (await h.call('rack-1', 'POST', '/internal/offer', offerBody({ run: noRunner }))).data.error,
    'OFFER_RUN',
  );
  assert.equal(
    (
      await h.call(
        'rack-1',
        'POST',
        '/internal/offer',
        offerBody({ run: { ...run, runner_nonce: 'e'.repeat(63) } }),
      )
    ).data.error,
    'OFFER_RUN',
  );
  assert.equal((await h.call('rack-1', 'POST', '/internal/offer', offerBody())).status, 200);
  const offer = await c.next('offer');
  assert.deepEqual(offer.run, { nonce: run.nonce, runner_nonce: run.runner_nonce, sha });
  assert.equal(validateServerFrame(offer), null);
  // The runner nonce is not stored on the Worker's offer record.
  const raw = await h.call('rack-1', 'GET', '/__t/storage');
  assert.ok(!raw.text.includes(run.runner_nonce));
});

test('welcome and cmd carry commands_until_ms; a window can be reopened', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
  const set = await h.open('own-1');
  const until = set.data.config.commands_until;
  const c = await h.hello('own-1', await h.mint('own-1'));
  assert.equal(c.welcome.commands_until_ms, until);
  assert.equal(c.welcome.cmd_output_per_s, 20);
  assert.equal(validateServerFrame(c.welcome), null);
  assert.equal(
    (await h.call('own-1', 'POST', '/admin/hosts/own-1/cmd', { shell: 'id' })).status,
    200,
  );
  const cmd = await c.next('cmd');
  assert.equal(cmd.commands_until_ms, until);
  assert.equal(validateServerFrame(cmd), null);
});

test('cmd_output has its own larger bucket; a real flood is still closed 1008', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
  await h.open('own-1');
  const c = await h.hello('own-1', await h.mint('own-1'));
  await h.call('own-1', 'POST', '/admin/hosts/own-1/cmd', { shell: 'yes' });
  const cmd = await c.next('cmd');
  const out = (i) =>
    c.send({
      type: 'cmd_output',
      cmd_id: cmd.cmd_id,
      seq: i,
      stream: 'stdout',
      data: 'x'.repeat(100),
    });
  for (let i = 0; i < 38; i++) out(i);
  await new Promise((r) => setTimeout(r, 300));
  assert.equal(await openSockets(h, 'own-1'), 1, '38 quick output frames are fine');
  for (let i = 0; i < 120; i++) out(100 + i);
  assert.equal(await c.code(), 1008);
});

test('seeder: concurrent registrations share ONE probe; another token meanwhile is PROBE_USED', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const { c, offer } = await ackedOffer(h, 'rack-1');
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'hold');
  reg(c, offer);
  reg(c, offer);
  await h.wait(
    async () => (await h.call('rack-1', 'GET', '/__t/probes')).data.length > 0,
    'probe in flight',
  );
  reg(c, offer, { path: '/zz9y8x7w/kf3.tar' });
  assert.equal((await c.next('seed_ack')).reason, 'PROBE_USED');
  await h.call('rack-1', 'POST', '/__t/release', 1, '');
  assert.equal((await c.next('seed_ack')).ok, true);
  assert.equal((await c.next('seed_ack')).ok, true);
  assert.equal(
    (await h.call('rack-1', 'GET', '/__t/probes')).data.length,
    1,
    'one probe, one name',
  );
});

test('seeder: warnings are stored and shown to the admin', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const { c, offer } = await ackedOffer(h, 'rack-1');
  reg(c, offer, { warnings: ['candidate unknown', 'staged copy differs'] });
  assert.equal((await c.next('seed_ack')).ok, true);
  const v = await h.view('rack-1');
  assert.deepEqual(v.seed.warnings, ['candidate unknown', 'staged copy differs']);
  assert.equal(v.seed.name, 'kf3.tar');
  reg(c, offer, { warnings: Array(6).fill('w') });
  assert.equal((await c.next('error')).code, 'BAD_FRAME');
});

test('consumer path: an offer can carry a seed built from a seeder record, never for borrowed', async (t) => {
  const h = await runtime(t);
  await h.put('seed-1', { profile: 'standard' });
  const { c: sc, offer: so } = await ackedOffer(h, 'seed-1');
  reg(sc, so);
  assert.equal((await sc.next('seed_ack')).ok, true);
  const record = (await h.view('seed-1')).seed;
  // A standard consumer gets the reference, built by the Worker from the record.
  await h.put('cons-1', { profile: 'standard' });
  const cc = await h.hello('cons-1', await h.mint('cons-1'));
  const made = await h.call(
    'cons-1',
    'POST',
    '/internal/offer',
    offerBody({ seed_from: 'seed-1' }),
  );
  assert.equal(made.status, 200, made.text);
  const offer = await cc.next('offer');
  assert.deepEqual(offer.assignment.seed, {
    url: record.url,
    fallback_url: record.fallback_url,
    sha256: record.sha256,
    size: record.size,
    name: record.name,
  });
  assert.equal(validateServerFrame(offer), null);
  // A planner cannot supply the seed itself; a missing seeder is unavailable; self is refused.
  assert.equal(
    (await h.call('cons-1', 'POST', '/internal/cancel', { offer_id: offer.offer_id })).status,
    200,
  );
  const planner = await h.call(
    'cons-1',
    'POST',
    '/internal/offer',
    offerBody({ request_id: 'r2', assignment: { ...assignment, seed: offer.assignment.seed } }),
  );
  assert.equal(planner.data.error, 'OFFER_ASSIGNMENT_KEYS');
  assert.equal(
    (
      await h.call(
        'cons-1',
        'POST',
        '/internal/offer',
        offerBody({ request_id: 'r3', seed_from: 'nobody' }),
      )
    ).data.error,
    'SEED_UNAVAILABLE',
  );
  assert.equal(
    (
      await h.call(
        'cons-1',
        'POST',
        '/internal/offer',
        offerBody({ request_id: 'r4', seed_from: 'cons-1' }),
      )
    ).data.error,
    'OFFER_SEED',
  );
  // A borrowed consumer never receives a seed.
  await h.put('bor-1', { profile: 'borrowed' });
  const bc = await h.hello('bor-1', await h.mint('bor-1'));
  const refused = await h.call(
    'bor-1',
    'POST',
    '/internal/offer',
    offerBody({ seed_from: 'seed-1' }),
  );
  assert.equal(refused.status, 403);
  assert.equal(refused.data.error, 'SEED_NOT_ALLOWED');
  assert.equal(bc.frames.filter((f) => f.type === 'offer').length, 0);
});

test('eviction between cmd_output and cmd_done: flushed output survives, a lost tail is flagged incomplete', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
  await h.open('own-1');
  const c = await h.hello('own-1', await h.mint('own-1'));
  const run1 = async (flushFirst) => {
    const r = await h.call('own-1', 'POST', '/admin/hosts/own-1/cmd', { shell: 'job' });
    const cmd = await c.next('cmd');
    c.send({
      type: 'cmd_output',
      cmd_id: cmd.cmd_id,
      seq: 0,
      stream: 'stdout',
      data: 'part one\n',
    });
    if (flushFirst)
      await new Promise((x) => setTimeout(x, 1700)); // the flush alarm fires
    else await new Promise((x) => setTimeout(x, 100));
    await h.call('own-1', 'GET', '/__t/evict'); // the object is rebuilt: in-memory state is gone
    c.send({ type: 'cmd_done', cmd_id: cmd.cmd_id, rc: 0 });
    await h.wait(
      async () => (await h.view('own-1')).commands.find((x) => x.cmd_id === r.data.cmd_id)?.done,
      'done',
    );
    return (await h.view('own-1')).commands.find((x) => x.cmd_id === r.data.cmd_id);
  };
  const flushed = await run1(true);
  assert.equal(flushed.output, 'part one\n');
  assert.ok(!flushed.incomplete, 'the alarm flushed it before the eviction');
  const lost = await run1(false);
  assert.equal(lost.done, true);
  assert.equal(
    lost.incomplete,
    true,
    'a tail that may have been lost is never presented as complete',
  );
  assert.equal(lost.dirty_since, undefined);
});

test('enrol: the command parses with bootstrap.py (flag set copied from the daemon README)', async (t) => {
  const prefix = 'P'.repeat(40);
  const h = await runtime(t, {
    HOST_DAEMON_COMMIT: COMMIT,
    BROKER_PATH_PREFIX: prefix,
    HOST_ACCESS_ID: 'abcdef0123456789.access',
  });
  await h.put('rack-1');
  const { command } = (await enrol(h, 'rack-1')).data;
  const [comment, line] = command.split('\n');
  assert.match(comment, /^# .*Access client SECRET.*accessfile/);
  assert.equal(command.split('\n').length, 2);
  const run = line.split(' && ').at(-1);
  assert.match(run, /^python3 kfhost-bootstrap\.py /);
  // bootstrap.py (PR #346): required --repo --commit --worker-url --box-id --token-file; optional
  // --access-id --access-secret-file --work-dir --dry-run (every flag takes a value except --dry-run).
  const tokens = run.split(' ').slice(2);
  const flags = {};
  for (let i = 0; i < tokens.length; i += 2) flags[tokens[i]] = tokens[i + 1];
  const known = [
    '--repo',
    '--commit',
    '--worker-url',
    '--box-id',
    '--token-file',
    '--access-id',
    '--access-secret-file',
    '--work-dir',
  ];
  assert.ok(
    Object.keys(flags).every((f) => known.includes(f)),
    JSON.stringify(flags),
  );
  for (const required of ['--repo', '--commit', '--worker-url', '--box-id', '--token-file'])
    assert.ok(flags[required], required);
  assert.equal(flags['--repo'], CANONICAL_REPO);
  assert.equal(flags['--commit'], COMMIT);
  assert.equal(flags['--access-id'], 'abcdef0123456789.access');
  assert.equal(flags['--access-secret-file'], 'accessfile');
  assert.equal(flags['--token-file'], 'kfhost.token');
  assert.ok(
    !command.includes('ACCESS_SECRET') || /<ACCESS_SECRET>/.test(comment),
    'only a placeholder for the secret',
  );
  // Without HOST_ACCESS_ID the command carries a placeholder the admin replaces.
  const h2 = await runtime(t, { HOST_DAEMON_COMMIT: COMMIT });
  await h2.put('rack-2');
  const c2 = (await enrol(h2, 'rack-2')).data.command;
  assert.match(c2, /--access-id <ACCESS_ID> /);
  // A repo allowlist that is not on github.com cannot be enrolled (the bootstrap URL is raw github).
  const h3 = await runtime(t, {
    HOST_DAEMON_COMMIT: COMMIT,
    HOST_REPO_ALLOWLIST: JSON.stringify(['https://git.example/x']),
  });
  await h3.put('rack-3');
  assert.equal((await enrol(h3, 'rack-3')).status, 503);
});
