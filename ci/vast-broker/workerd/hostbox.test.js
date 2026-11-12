import test from 'node:test';
import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';
import { env } from '../test/helpers.js';
import { SEED_PROBE_CONTENT } from '../src/hosts.js';

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
        this.probes = [];
        this.mode = 'ok';
        this.fetcher = async (url) => {
          this.probes.push(String(url));
          if (this.mode === 'first-fails' && String(url).includes('sslip.io')) return new Response('no', {status: 522});
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
  seeder: { enabled: true, remote_ip: '203.0.113.9', external_port: 40022, inner_port: 8080 },
  window: null,
  opportunistic: false,
  notes: '',
  ...patch,
});
const caps = {
  gpu_name: 'RTX 4070',
  compute_cap: '8.9',
  vram_gb: 12,
  kvm: true,
  display: true,
  can_swap_driver: true,
  bogus: 1,
};
const sha = 'a'.repeat(40);
const assignment = {
  repo: 'reindertpelsma/kayfabe',
  commit: sha,
  tree_key: 'kf-tree-v2:' + 'b'.repeat(16),
};
const run = {
  run: '123',
  attempt: '1',
  nonce: 'c'.repeat(32),
  reportExpiry: Math.floor(Date.now() / 1000) + 3600,
};
const offerBody = (extra = {}) => ({
  request_id: 'req-1',
  run,
  assignment,
  deadline: Date.now() + 3600000,
  ...extra,
});

const runtime = async (t) => {
  const mf = new Miniflare(
    convertV4MiniflareOptions({
      modules: true,
      script: bundle.outputFiles[0].text,
      compatibilityDate: '2026-10-23',
      bindings: { ...env, RENTING_ENABLED: 'false' },
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
    const r = await mf.dispatchFetch('https://broker.example/hosts/' + id + '/ws', {
      headers: {
        Upgrade: 'websocket',
        ...(token ? { Authorization: 'Bearer ' + token } : {}),
        ...headers,
      },
    });
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
      return Promise.race([closedP, new Promise((res) => setTimeout(() => res('open'), 1500))]);
    };
    return { sock, frames, next, send, code, closedP };
  };
  const hello = async (id, token, extra = {}) => {
    const c = await connect(id, token);
    c.send({ type: 'hello', box_id: id, version: '0.1', capabilities: caps, ...extra });
    c.welcome = await c.next('welcome');
    return c;
  };
  const view = async (id) => (await call(id, 'GET', '/admin/hosts/' + id)).data;
  return { mf, stub, call, put, mint, connect, hello, view };
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
  const g = await h.mf.dispatchFetch('https://broker.example/admin/hosts');
  assert.equal(g.status, 404);
  await g.text();
});

test('ws: no config, no token, wrong token and malformed token all close 4401', async (t) => {
  const h = await runtime(t);
  assert.equal(await (await h.connect('ghost', 'kfh_' + 'A'.repeat(43))).code(), 4401);
  await h.put('rack-1');
  assert.equal(await (await h.connect('rack-1', undefined)).code(), 4401, 'no token minted yet');
  const token = await h.mint('rack-1');
  assert.equal(await (await h.connect('rack-1', undefined)).code(), 4401);
  assert.equal(await (await h.connect('rack-1', 'kfh_' + 'A'.repeat(43))).code(), 4401);
  assert.equal(await (await h.connect('rack-1', 'garbage')).code(), 4401);
  assert.equal(await (await h.connect('rack-1', token + 'x')).code(), 4401);
  // Another box's valid token does not open this box.
  await h.put('rack-2');
  const other = await h.mint('rack-2');
  assert.equal(await (await h.connect('rack-1', other)).code(), 4401);
});

test('ws: a plain GET is refused and a bad box id is not routed', async (t) => {
  const h = await runtime(t);
  const r = await h.mf.dispatchFetch('https://broker.example/hosts/rack-1/ws');
  assert.equal(r.status, 426);
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
  const c = await h.hello('rack-1', token, { profile: 'owned' });
  assert.deepEqual(c.welcome, {
    v: 1,
    type: 'welcome',
    box_id: 'rack-1',
    profile: 'borrowed',
    heartbeat_s: 15,
  });
  const v = await h.view('rack-1');
  assert.equal(v.connected, true);
  assert.equal(v.state, 'idle');
  assert.equal('bogus' in v.capabilities, false);
  assert.equal(v.capabilities.compute_cap, '8.9');
  assert.equal(v.version, '0.1');
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

test('replace-on-reconnect: the newer connection wins, the old one closes 4409', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const token = await h.mint('rack-1');
  const first = await h.hello('rack-1', token);
  const second = await h.hello('rack-1', token);
  assert.equal(await first.code(), 4409);
  assert.equal((await h.view('rack-1')).connected, true);
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  assert.equal((await second.next('offer')).type, 'offer');
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
  c.send({ type: 'nack', offer_id: 'nope' });
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
    [{ deadline: 1 }, 'OFFER_RUN'],
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

test('commands: owned only; container and borrowed refuse, nothing is sent', async (t) => {
  const h = await runtime(t);
  for (const profile of ['container', 'borrowed']) {
    const id = 'x-' + profile;
    await h.put(id, { profile });
    const c = await h.hello(id, await h.mint(id));
    for (const body of [{ argv: ['id'] }, { shell: 'id' }]) {
      const r = await h.call(id, 'POST', '/admin/hosts/' + id + '/cmd', body);
      assert.equal(r.status, 403);
      assert.equal(r.data.error, 'CMD_NOT_ALLOWED');
    }
    assert.equal(c.frames.filter((f) => f.type === 'cmd').length, 0);
    // A box that claims command output anyway is dropped.
    c.send({ type: 'cmd_output', cmd_id: 'x', seq: 0, stream: 'stdout', data: 'hi' });
    assert.equal(await c.code(), 1008);
  }
});

test('commands: owned box gets the frame, output and exit code are logged, bounded', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
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
  c.send({ type: 'cmd_output', cmd_id: cmd.cmd_id, seq: 0, stream: 'stdout', data: 'GPU 0\n' });
  c.send({
    type: 'cmd_output',
    cmd_id: cmd.cmd_id,
    seq: 1,
    stream: 'stderr',
    data: 'x'.repeat(40000),
  });
  c.send({ type: 'cmd_done', cmd_id: cmd.cmd_id, rc: 0 });
  for (let i = 0; i < 100 && !(await h.view('own-1')).commands[0].done; i++)
    await new Promise((x) => setTimeout(x, 20));
  const log = (await h.view('own-1')).commands[0];
  assert.equal(log.rc, 0);
  assert.ok(log.output.startsWith('GPU 0\n'));
  assert.ok(log.output.length <= 32768);
  assert.equal(log.truncated, true);
});

test('commands: refused while the box is not connected', async (t) => {
  const h = await runtime(t);
  await h.put('own-1');
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

test('seeder: probes only the configured address; the box cannot choose another', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1');
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const offer = await c.next('offer');
  const artifact = { name: 'kf3.tar', sha256: 'd'.repeat(64), size: 1234 };
  const tok = 'S'.repeat(24);
  // Not acked yet: refused.
  c.send({
    type: 'seed_register',
    offer_id: offer.offer_id,
    artifact,
    path: '/' + tok + '/kf3.tar',
  });
  assert.deepEqual(await c.next('seed_ack'), {
    v: 1,
    type: 'seed_ack',
    ok: false,
    reason: 'BAD_REGISTRATION',
  });
  c.send({ type: 'ack', offer_id: offer.offer_id });
  await new Promise((r) => setTimeout(r, 100));
  // The box names its own address in several fields; none is used.
  c.send({
    type: 'seed_register',
    offer_id: offer.offer_id,
    artifact,
    path: '/' + tok + '/kf3.tar',
    ip: '198.51.100.1',
    port: 1,
    url: 'http://198.51.100.1:1/x',
  });
  const ok = await c.next('seed_ack');
  assert.equal(ok.ok, true);
  const probes = (await h.call('rack-1', 'GET', '/__t/probes')).data;
  assert.deepEqual(probes, ['http://203-0-113-9.sslip.io:40022/' + tok + '/probe.txt']);
  const v = await h.view('rack-1');
  assert.equal(v.seed.url, 'http://203.0.113.9:40022/' + tok + '/kf3.tar');
  assert.equal(v.seed.sha256, 'd'.repeat(64));
  assert.equal(v.seed.size, 1234);
  assert.equal(v.seed.fallback_url, 'http://203-0-113-9.sslip.io:40022/' + tok + '/kf3.tar');
  // Wrong content, a redirect and a path that disagrees with the artifact are refused.
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'wrong');
  c.send({
    type: 'seed_register',
    offer_id: offer.offer_id,
    artifact,
    path: '/' + tok + '/kf3.tar',
  });
  assert.equal((await c.next('seed_ack')).reason, 'WRONG_CONTENT');
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'redirect');
  c.send({
    type: 'seed_register',
    offer_id: offer.offer_id,
    artifact,
    path: '/' + tok + '/kf3.tar',
  });
  assert.equal((await c.next('seed_ack')).reason, 'REDIRECT');
  c.send({
    type: 'seed_register',
    offer_id: offer.offer_id,
    artifact,
    path: '/' + tok + '/other.tar',
  });
  assert.equal((await c.next('seed_ack')).reason, 'BAD_REGISTRATION');
  c.send({ type: 'seed_register', offer_id: offer.offer_id, artifact, path: '/../x/kf3.tar' });
  assert.equal((await c.next('seed_ack')).reason, 'BAD_REGISTRATION');
});

test('seeder disabled in the admin config: refused without any fetch', async (t) => {
  const h = await runtime(t);
  await h.put('rack-1', { seeder: { enabled: false } });
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const offer = await c.next('offer');
  c.send({ type: 'ack', offer_id: offer.offer_id });
  await new Promise((r) => setTimeout(r, 100));
  c.send({
    type: 'seed_register',
    offer_id: offer.offer_id,
    artifact: { name: 'a.tar', sha256: 'd'.repeat(64), size: 1 },
    path: '/' + 'S'.repeat(24) + '/a.tar',
  });
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
    remote_ip: '203.0.113.9',
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
  const c = await h.hello('rack-1', await h.mint('rack-1'));
  await h.call('rack-1', 'POST', '/internal/offer', offerBody());
  const o = await c.next('offer');
  c.send({ type: 'ack', offer_id: o.offer_id });
  await new Promise((r) => setTimeout(r, 100));
  const tok = 'S'.repeat(24);
  c.send({
    type: 'seed_register',
    offer_id: o.offer_id,
    artifact: { name: 'a.tar', sha256: 'd'.repeat(64), size: 1 },
    path: '/' + tok + '/a.tar',
  });
  assert.equal((await c.next('seed_ack')).ok, true);
  const v = await h.view('rack-1');
  assert.equal(v.seed.fallback_url, 'http://203-0-113-9.traefik.me:40022/' + tok + '/a.tar');
  assert.equal(v.seed.url, 'http://203.0.113.9:40022/' + tok + '/a.tar');
  assert.deepEqual((await h.call('rack-1', 'GET', '/__t/probes')).data, [
    'http://203-0-113-9.sslip.io:40022/' + tok + '/probe.txt',
    'http://203-0-113-9.traefik.me:40022/' + tok + '/probe.txt',
  ]);
  // Both names failing is "cannot open a port".
  await h.call('rack-1', 'POST', '/__t/mode', 1, 'wrong');
  c.send({
    type: 'seed_register',
    offer_id: o.offer_id,
    artifact: { name: 'a.tar', sha256: 'd'.repeat(64), size: 1 },
    path: '/' + tok + '/a.tar',
  });
  assert.equal((await c.next('seed_ack')).ok, false);
});
