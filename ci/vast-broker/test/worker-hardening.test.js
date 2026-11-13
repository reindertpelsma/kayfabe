import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile, cp } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { MockCloud, env, clock, setup, nonce } from './helpers.js';
import { configuration } from '../src/policy.js';
import { Vast } from '../src/vast.js';
import { sha256Hex } from '../src/tokens.js';
import * as routes from '../src/routes.js';
import * as publiclog from '../src/publiclog.js';
import * as index from '../src/index.js';
// A symbol this change adds is looked up by name, so that against the code before the change the
// tests fail one by one (RED) instead of the whole file failing to import.
const missing = (name) => () => {
  throw Error('missing export ' + name);
};
const harden = routes.harden ?? missing('harden');
const publicHeaders = publiclog.publicHeaders ?? missing('publicHeaders');
const ipRateKey = index.ipRateKey ?? missing('ipRateKey');
const { putPublic } = publiclog;
const { createGateway } = index;

// FALSIFIER (worker hardening review, 2026-11-13, findings F4 F6 F8 F10): a box that relabels its
// own instance escapes teardown; a log or Worker response is served without nosniff; the deploy
// helper uploads a module it did not hash; a box-reported PASS decays a host's fault history; the
// reaper default is below the hard limit; the upgrade limiter is keyed per IPv6 address or locks
// real daemons out once its table is full. Each test fails against the code before this change.

// ---- F4: destroy by the proven id --------------------------------------------------------------
const broker = fileURLToPath(new URL('../', import.meta.url));
async function relabelled(id, over = {}) {
  const cloud = new MockCloud();
  const script = 'owned nonce ' + id;
  const record = {
    id: String(id),
    label: 'kf-ci-' + id + '-1',
    idSource: 'reconcile',
    proofVersion: 1,
    scriptHash: await sha256Hex(script),
    ...over,
  };
  cloud.instances.set(String(id), {
    id,
    label: 'renamed-by-the-box',
    onstart: script,
    image_uuid: 'x',
  });
  return { cloud, record, vast: new Vast(cloud.fetch, configuration(env), 'mock-key') };
}
for (const id of [900, 901]) {
  test('F4 destroy deletes a reconcile-proven instance whose label changed ' + id, async () => {
    const { cloud, record, vast } = await relabelled(id);
    await vast.destroy(record);
    assert.deepEqual(cloud.deletes, [String(id)]);
    assert.equal(cloud.instances.has(String(id)), false);
  });
  test('F4 destroy refuses a response for another id ' + id, async () => {
    const { cloud, record, vast } = await relabelled(id);
    cloud.instances.set(String(id), { ...cloud.instances.get(String(id)), id: id + 1 });
    await assert.rejects(vast.destroy(record), (e) => e.state === 'OWNERSHIP_MISMATCH');
    assert.deepEqual(cloud.deletes, []);
  });
  test('F4 destroy refuses a ledger label outside kf-ci- ' + id, async () => {
    const { cloud, record, vast } = await relabelled(id, { label: 'owner-dev-' + id });
    await assert.rejects(vast.destroy(record), (e) => e.state === 'OWNERSHIP_MISMATCH');
    assert.deepEqual(cloud.deletes, []);
  });
  test('F4 destroy still needs the onstart proof of a proofVersion 1 record ' + id, async () => {
    const { cloud, record, vast } = await relabelled(id, { scriptHash: await sha256Hex('other') });
    await assert.rejects(vast.destroy(record), (e) => e.state === 'OWNERSHIP_MISMATCH');
    assert.deepEqual(cloud.deletes, []);
  });
  test('F4 plain inspect still refuses a relabelled reconcile record ' + id, async () => {
    const { record, vast } = await relabelled(id);
    await assert.rejects(vast.inspect(record), (e) => e.state === 'OWNERSHIP_MISMATCH');
  });
}

// ---- F4: the reaper ----------------------------------------------------------------------------
let now = 1800000000000;
const epoch = (minutes) => (now - minutes * 60000) / 1000;
async function reaperHarness(rows, { kv, vars } = {}) {
  const { createReaper } = await import('../../vast-reaper/worker.js');
  const records = new Map(rows.map((x) => [String(x.id), { ...x }]));
  const deletes = [],
    logs = [];
  const worker = createReaper({
    now: () => now,
    report: (x) => logs.push(x),
    fetcher: async (input, init = {}) => {
      const url = new URL(input);
      if (url.pathname === '/api/v1/instances/')
        return Response.json({ success: true, instances: [...records.values()] });
      const match = /^\/api\/v0\/instances\/([1-9][0-9]*)\/$/.exec(url.pathname);
      assert.ok(match);
      assert.equal(init.method, 'DELETE');
      deletes.push(match[1]);
      records.delete(match[1]);
      return Response.json({ success: true });
    },
  });
  const bindings = { REAPER_VAST_API_KEY: 'k', REAPER_MARGIN_MINUTES: '15', ...vars };
  if (kv) bindings.REAPER_SEEN = kv;
  return { worker, records, deletes, logs, env: bindings };
}
const memoryKv = () => {
  const data = new Map();
  return {
    data,
    async get(k) {
      return data.get(k) ?? null;
    },
    async put(k, v) {
      data.set(k, v);
    },
  };
};
const row = (id, minutes, label = 'kf-ci-' + id + '-1') => ({
  id,
  label,
  start_date: epoch(minutes),
});
for (const id of [1, 2]) {
  test('F4 reaper deletes an id it saw under the label after the label changed ' + id, async () => {
    const kv = memoryKv();
    const h = await reaperHarness([row(id, 10)], { kv, vars: { REAPER_HARD_MINUTES: '300' } });
    await h.worker.sweep(h.env); // young: remembered, not deleted
    assert.deepEqual(h.deletes, []);
    h.records.get(String(id)).label = 'renamed';
    now += 400 * 60000; // the same start_date, now far past the limit
    await h.worker.sweep(h.env);
    assert.deepEqual(h.deletes, [String(id)]);
  });
  test('F4 reaper never deletes an id it never saw under the label ' + id, async () => {
    const kv = memoryKv();
    const h = await reaperHarness([row(id, 999, 'owner-dev')], {
      kv,
      vars: { REAPER_HARD_MINUTES: '300' },
    });
    await h.worker.sweep(h.env);
    assert.deepEqual(h.deletes, []);
    assert.deepEqual(h.logs, [{ event: 'reaper_complete', deleted: 0 }]);
  });
  test('F4 reaper forgets an id that is gone, so a reused id is not ours ' + id, async () => {
    const kv = memoryKv();
    const h = await reaperHarness([row(id, 10)], { kv, vars: { REAPER_HARD_MINUTES: '300' } });
    await h.worker.sweep(h.env);
    h.records.clear();
    await h.worker.sweep(h.env);
    assert.equal(kv.data.get('seen'), '{}');
    h.records.set(String(id), row(id, 999, 'owner-dev'));
    await h.worker.sweep(h.env);
    assert.deepEqual(h.deletes, []);
  });
  test('F4 reaper without the binding still decides by label alone ' + id, async () => {
    const h = await reaperHarness([row(id, 400, 'renamed')], {
      vars: { REAPER_HARD_MINUTES: '300' },
    });
    await h.worker.sweep(h.env);
    assert.deepEqual(h.deletes, []);
  });
}
// ---- F10: the reaper default is above the hard limit ----------------------------------------------
for (const [age, deleted] of [
  [200, false],
  [314, false],
  [316, true],
]) {
  test('F10 reaper code default is the 300 minute hard limit plus margin, age ' + age, async () => {
    const h = await reaperHarness([row(7, age)]);
    await h.worker.sweep(h.env); // no REAPER_HARD_MINUTES: the code default applies
    assert.equal(h.deletes.length === 1, deleted);
  });
}

// ---- F6: headers ---------------------------------------------------------------------------------
test('F6 public log objects carry a fixed inline filename and the stored type', () => {
  assert.deepEqual(publicHeaders('1-1/log.txt', 'text/plain; charset=utf-8'), {
    contentType: 'text/plain; charset=utf-8',
    contentDisposition: 'inline; filename="log.txt"',
    cacheControl: 'no-transform',
  });
  assert.equal(
    publicHeaders('1-1/report.json', 'application/json; charset=utf-8').contentDisposition,
    'inline; filename="report.json"',
  );
});
for (const [key, type] of [
  ['1-1/log.txt', 'text/plain; charset=utf-8'],
  ['1-1/report.json', 'application/json; charset=utf-8'],
])
  test('F6 putPublic writes those headers to R2 ' + key, async () => {
    const calls = [];
    await putPublic({ LOGS_BUCKET: { put: async (...a) => calls.push(a) } }, key, 'x', type);
    assert.deepEqual(calls[0][2], { httpMetadata: publicHeaders(key, type) });
  });
for (const [name, response] of [
  ['text', () => new Response('box text <script>', { headers: { 'Content-Type': 'text/plain' } })],
  ['html', () => new Response('<h1>', { headers: { 'Content-Type': 'text/html' } })],
  ['untyped', () => new Response('x')],
  ['json', () => Response.json({ a: 1 })],
])
  test('F6 every response gets nosniff: ' + name, async () => {
    const out = harden(response());
    assert.equal(out.headers.get('X-Content-Type-Options'), 'nosniff');
    assert.equal(out.status, 200);
    const text = name === 'json' ? '{"a":1}' : undefined;
    if (text) assert.equal(await out.text(), text);
    else await out.arrayBuffer();
  });
for (const type of ['text/plain', 'text/html; charset=utf-8', null]) {
  test('F6 a non-JSON response gets a deny-all CSP and an attachment disposition ' + type, () => {
    const out = harden(new Response('x', type ? { headers: { 'Content-Type': type } } : {}));
    assert.equal(out.headers.get('Content-Security-Policy'), "default-src 'none'; sandbox");
    assert.match(out.headers.get('Content-Disposition'), /^attachment;/);
  });
}
test('F6 a JSON response is not turned into a download', () => {
  const out = harden(Response.json({}));
  assert.equal(out.headers.get('Content-Disposition'), null);
});
test('F6 a WebSocket upgrade is returned untouched', () => {
  const upgrade = { status: 101, webSocket: {}, headers: new Headers() };
  assert.equal(harden(upgrade), upgrade);
});
for (const path of ['/nope', '/status/1-1'])
  test('F6 the public gateway answers with nosniff even on refusals ' + path, async () => {
    const response = await createGateway().fetch(new Request('https://broker.example' + path), {
      ...env,
      BROKER_PATH_PREFIX: 'p'.repeat(40),
    });
    assert.equal(response.headers.get('X-Content-Type-Options'), 'nosniff');
    assert.match(response.headers.get('Content-Security-Policy') ?? '', /default-src 'none'/);
    await response.arrayBuffer();
  });

// ---- F10: a box-reported PASS does not decay the fault history -----------------------------------
const seedFault = (h) =>
  h.storage.put('provider-machine:100001', {
    faults: [{ at: clock, weight: 2, reason: 'GPU_START_ERROR' }],
  });
for (const [verdict, weight] of [
  ['PASS', 2],
  ['PASS', 3],
  ['FAIL', 2],
]) {
  test('F10 a box result (' + verdict + ') never decays the fault history ' + weight, async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    await h.storage.put('provider-machine:100001', {
      faults: [{ at: clock, weight, reason: 'GPU_START_ERROR' }],
    });
    const response = await h.report({ seq: 1, kind: 'result', result: { nonce, verdict } });
    assert.equal(response.status, 200);
    const history = await h.storage.get('provider-machine:100001');
    assert.equal(history.faults[0].weight, weight);
    assert.equal(history.success, undefined);
  });
}
for (const factor of [undefined]) {
  test('F10 the owner admin path decays the history once per call', async () => {
    const h = await setup();
    await seedFault(h);
    await h.service.adminProviderSuccess(100001, 5);
    const history = await h.storage.get('provider-machine:100001');
    assert.ok(history.faults[0].weight < 2 && history.faults[0].weight > 0, String(factor));
    assert.match(history.success, /^admin-/);
  });
}

// ---- F10: the host-upgrade rate limit ------------------------------------------------------------
test('F10 ipRateKey keys IPv6 by its /64 and leaves IPv4 alone', () => {
  assert.equal(ipRateKey('203.0.113.9'), '203.0.113.9');
  assert.equal(ipRateKey('unknown'), 'unknown');
  const a = ipRateKey('2001:db8:1:2::1');
  assert.equal(a, '2001:0db8:0001:0002::/64');
  assert.equal(ipRateKey('2001:DB8:1:2:ffff:ffff:ffff:ffff'), a);
  assert.equal(ipRateKey('2001:db8:1:2:0:0:0:0'), a);
  assert.notEqual(ipRateKey('2001:db8:1:3::1'), a);
  assert.equal(ipRateKey('::ffff:203.0.113.9'), '::ffff:203.0.113.9');
});
function upgradeEnv() {
  const wake = [];
  return {
    wake,
    env: {
      ...env,
      RENTING_ENABLED: 'false',
      HOSTS: {
        idFromName: (n) => n,
        get: () => ({ fetch: async () => (wake.push(1), new Response('woken')) }),
      },
    },
  };
}
const upgrade = (ip) =>
  new Request('https://broker.example/hosts/rack-1/ws', {
    headers: {
      Upgrade: 'websocket',
      Authorization: 'Bearer kfh_' + 'A'.repeat(43),
      'CF-Connecting-IP': ip,
    },
  });
test('F10 rotating IPv6 addresses inside one /64 share one budget of 30', async () => {
  const { env: e } = upgradeEnv();
  const gateway = createGateway({ now: () => 1800000000000 });
  const codes = [];
  for (let i = 0; i < 33; i++)
    codes.push(
      (await gateway.fetch(upgrade('2001:db8:1:2:0:0:0:' + (i + 1).toString(16)), e)).status,
    );
  assert.equal(codes.filter((c) => c === 429).length, 3);
  assert.equal(codes.indexOf(429), 30);
});
for (const real of ['198.51.100.7', '2001:db8:77:1::5'])
  test('F10 a full table evicts instead of locking out a new address ' + real, async () => {
    const { env: e, wake } = upgradeEnv();
    const gateway = createGateway({ now: () => 1800000000000 });
    for (let i = 0; i < 1100; i++)
      await gateway.fetch(upgrade('10.' + (i >> 8) + '.' + (i & 255) + '.1'), e);
    const before = wake.length;
    const answer = await gateway.fetch(upgrade(real), e);
    assert.equal(answer.status, 200);
    assert.equal(wake.length, before + 1);
  });

// ---- F10: /internal/seed after a seeder is disabled is covered by workerd/hardening.test.js -------

// ---- F8: the deploy helper ------------------------------------------------------------------------
async function deployFixture(emit) {
  const root = await mkdtemp(join(tmpdir(), 'f8-deploy-'));
  const site = join(root, 'broker');
  await mkdir(site, { recursive: true });
  await cp(broker + 'wrangler.toml', join(site, 'wrangler.toml'));
  const bundle = 'export default {};\n';
  const digest = createHash('sha256').update(bundle).digest('hex');
  await writeFile(join(site, 'deployment.json'), JSON.stringify({ bundle_sha256: digest }));
  const toml = (await readFile(join(site, 'wrangler.toml'), 'utf8')).replace(
    /^DEPLOYED_BUNDLE_SHA256 = ".*"$/m,
    'DEPLOYED_BUNDLE_SHA256 = "' + digest + '"',
  );
  await writeFile(join(site, 'wrangler.toml'), toml);
  const log = join(root, 'wrangler.log');
  await writeFile(
    join(root, 'wrangler'),
    '#!/usr/bin/env node\nconst fs=require("fs");const a=process.argv.slice(2);\n' +
      'if(a[0]==="--version"){console.log("4.152.0");process.exit(0);}\n' +
      'fs.appendFileSync(' +
      JSON.stringify(log) +
      ',JSON.stringify(a)+"\\n");\n' +
      'const o=a.indexOf("--outdir");\n' +
      'if(o>=0){fs.writeFileSync(a[o+1]+"/index.js",' +
      JSON.stringify(bundle) +
      ');' +
      emit +
      '}\n',
    { mode: 0o700 },
  );
  const vars = join(root, 'vars.env');
  await writeFile(vars, 'OWNER_GITHUB_ID=229738317\n');
  const owner = /^OWNER_GITHUB_ID = "(.*)"$/m.exec(toml)[1];
  await writeFile(vars, 'OWNER_GITHUB_ID=' + owner + '\n');
  const run = spawnSync(process.execPath, [broker + 'deploy.mjs', vars], {
    encoding: 'utf8',
    timeout: 20000,
    env: { ...process.env, KF_DEPLOY_ROOT: site, WRANGLER: join(root, 'wrangler') },
  });
  let calls = [];
  try {
    calls = (await readFile(log, 'utf8')).trim().split('\n').map(JSON.parse);
  } catch {}
  await rm(root, { recursive: true });
  return { run, calls, digest };
}
for (const extra of ['other.js', 'wasm.wasm'])
  test('F8 deploy refuses a file beside the hashed bundle ' + extra, async () => {
    const { run, calls } = await deployFixture(
      'fs.writeFileSync(a[o+1]+"/' + extra + '","export const x=1;");',
    );
    assert.equal(run.status, 2, run.stdout + run.stderr);
    assert.match(run.stderr, /beyond the hashed bundle/);
    assert.equal(calls.filter((c) => !c.includes('--outdir')).length, 0, 'nothing uploaded');
  });
test('F8 wrangler own files (README.md, index.js.map) are accepted', async () => {
  const { run } = await deployFixture(
    'fs.writeFileSync(a[o+1]+"/README.md","r");fs.writeFileSync(a[o+1]+"/index.js.map","{}");',
  );
  assert.equal(run.status, 0, run.stdout + run.stderr);
});
test('F8 the Worker reports the hash of the uploaded bytes, not a toml string', async () => {
  const { run, calls, digest } = await deployFixture('');
  assert.equal(run.status, 0, run.stdout + run.stderr);
  const upload = calls.find((c) => !c.includes('--outdir'));
  const at = upload.indexOf('DEPLOYED_BUNDLE_SHA256:' + digest);
  assert.ok(at > 0 && upload[at - 1] === '--var', JSON.stringify(upload));
});
