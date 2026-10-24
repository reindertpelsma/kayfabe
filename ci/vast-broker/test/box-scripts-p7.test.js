import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import https from 'node:https';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { mkdtemp, mkdir, copyFile, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { setup, callbackToken, sampleResult } from './helpers.js';
import { DEFAULTS } from '../src/defaults.js';
import { onstart } from '../src/onstart.js';
// Box protocol 7, end to end: the repo-side scripts (scripts/ci/vast_onstart.sh, vast_report.py,
// kf-box-kill, kf-box-blob-url) run for real against the real in-process Worker. Only the
// transport is faked: a local TLS listener hands each request to the Worker's fetch. The
// environment is the one the Worker puts in the Vast create call, not a hand-written copy.
// FALSIFIER: a box that cannot report, kill, mint a blob URL or be accepted by the Worker
// (token shape, Access pair, result schema, sequence rules), or that leaks a secret.
const scripts = new URL('../../../scripts/ci/', import.meta.url).pathname;
const access = {
  BOX_ACCESS_ID: '0123456789abcdef0123456789abcdef.access',
  BOX_ACCESS_SECRET: 'f0e1d2c3b4a5968778695a4b3c2d1e0f'.repeat(2),
};
const FAST =
  '--poll 0.05 --flush 0.2 --min-gap 0.05 --retry-base 0.1 --retry-cap 0.4 --final-budget 20';
const RUN = '123-1';
let cert;
test.before(async () => {
  const dir = await mkdtemp(tmpdir() + '/kf-p7-cert-');
  execFileSync(
    'openssl',
    ['req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1', '-nodes'].concat([
      ...['-keyout', dir + '/k.pem', '-out', dir + '/c.pem', '-days', '2'],
      ...['-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1'],
    ]),
    { stdio: 'ignore' },
  );
  cert = { dir, cert: dir + '/c.pem', key: readFileSync(dir + '/k.pem') };
  cert.pem = readFileSync(cert.cert);
});
test.after(async () => cert && (await rm(cert.dir, { recursive: true })));
// A rented run: the Worker's harness plus a TLS front that forwards to it, with every request
// recorded. The mock clock moves one second per request (the Worker rate-limits by its clock).
async function rented(overrides = {}, extra = {}) {
  const h = await setup(overrides);
  assert.equal((await h.call()).status, 200);
  const create = h.cloud.calls.find((c) => c.url.includes('/asks/')).body;
  const prefix = h.bindings.BROKER_PATH_PREFIX;
  const seen = [];
  const server = https.createServer({ key: cert.key, cert: cert.pem }, async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = Buffer.concat(chunks).toString('utf8');
    const headers = new Headers();
    for (const name of ['authorization', 'content-type', 'cf-access-client-id'])
      if (req.headers[name]) headers.set(name, req.headers[name]);
    if (req.headers['cf-access-client-secret'])
      headers.set('cf-access-client-secret', req.headers['cf-access-client-secret']);
    const entry = { method: req.method, path: req.url, headers: req.headers, body };
    seen.push(entry);
    h.time.value += 1000;
    const path = req.url.slice(prefix.length + 1);
    const response = await h.service.fetch(
      new Request(h.bindings.WORKER_URL + path, {
        method: req.method,
        headers,
        ...(req.method === 'POST' ? { body } : {}),
      }),
    );
    entry.status = response.status;
    entry.answer = await response.text();
    res.writeHead(response.status, { 'Content-Type': 'application/json' });
    res.end(entry.answer);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = 'https://127.0.0.1:' + server.address().port + '/' + prefix;
  const dir = await mkdtemp(tmpdir() + '/kf-p7-box-');
  const box = {
    h,
    seen,
    dir,
    url,
    prefix,
    // The environment exactly as the Worker created it, with only the endpoint redirected.
    env: { ...create.env, KF_BROKER_URL: url, ...extra },
    onstartText: create.onstart,
    reports: () => seen.filter((r) => r.path.includes('/report/')),
    posted: () => box.reports().map((r) => JSON.parse(r.body)),
    record: () => h.storage.get('record:' + RUN),
    async close() {
      await new Promise((resolve) => server.close(resolve));
      server.closeAllConnections?.();
      await rm(dir, { recursive: true });
    },
  };
  return box;
}
const baseEnv = () => ({
  PATH: process.env.PATH,
  HOME: process.env.HOME || '/tmp',
  LC_ALL: 'C.UTF-8',
  SSL_CERT_FILE: cert.cert,
  CURL_CA_BUNDLE: cert.cert,
});
function run(command, args, env, cwd) {
  return new Promise((resolve) => {
    const child = spawn(command, args, { env, cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '',
      err = '';
    child.stdout.on('data', (d) => (out += d));
    child.stderr.on('data', (d) => (err += d));
    const timer = setTimeout(() => child.kill('KILL'), 120000);
    child.on('close', (status) => {
      clearTimeout(timer);
      resolve({ status, out, err });
    });
  });
}
// A checkout holding the real helper scripts and a stand-in payload.
async function tree(box, payload) {
  const ci = box.dir + '/tree/scripts/ci';
  await mkdir(ci, { recursive: true });
  for (const name of ['vast_onstart.sh', 'vast_report.py', 'kf-box-kill', 'kf-box-blob-url'])
    await copyFile(scripts + name, ci + '/' + name);
  if (payload !== undefined) await writeFile(ci + '/vast_payload.sh', '#!/bin/bash\n' + payload);
  return box.dir + '/tree';
}
const dirs = (box) => ({
  KF_CI_WORKSPACE: box.dir + '/ws',
  KF_CI_RUN_DIR: box.dir + '/run',
  KF_CI_BIN_DIR: box.dir + '/bin',
  KF_CI_REPORTER_ARGS: FAST,
  KF_TEST_DUMP: box.dir + '/dump',
});
const runOnstart = async (box, payload, env = {}) => {
  const root = await tree(box, payload);
  await mkdir(box.dir + '/bin', { recursive: true });
  return run(
    'bash',
    [root + '/scripts/ci/vast_onstart.sh'],
    { ...baseEnv(), ...box.env, ...dirs(box), ...env },
    box.dir,
  );
};
const passDoc = (patch = {}) => ({ verdict: 'PASS', ...sampleResult(), ...patch });
const PASS_PAYLOAD =
  'env | grep "^KF_" | sort > "$KF_TEST_DUMP"\n' +
  `cat > "$KF_CI_WORKSPACE/kf-result.json" <<'EOF'\n${JSON.stringify(passDoc())}\nEOF\n`;
const resultOf = (box) => box.posted().find((p) => p.kind === 'result')?.result;
// ---- the files the Worker starts -------------------------------------------------------
test('p7 the repo has every file the Worker starts or reads', () => {
  assert.equal(DEFAULTS.onstartPath, 'scripts/ci/vast_onstart.sh');
  assert.equal(DEFAULTS.reporterPath, 'scripts/ci/vast_report.py');
  const text = onstart('a'.repeat(40), 'reindertpelsma/kayfabe');
  assert.ok(text.trim().endsWith('exec bash ' + DEFAULTS.onstartPath));
  for (const path of [
    DEFAULTS.onstartPath,
    DEFAULTS.reporterPath,
    'scripts/ci/vast_payload.sh',
    'scripts/ci/vast_payload.py',
    'scripts/ci/vast_setup.py',
    'scripts/ci/kf-box-kill',
    'scripts/ci/kf-box-blob-url',
  ])
    assert.ok(statSync(scripts + '../../' + path).isFile(), path);
  const source = readFileSync(scripts + 'vast_onstart.sh', 'utf8');
  assert.match(source, /^payload=scripts\/ci\/vast_payload\.sh$/m);
  assert.ok(!existsSync(scripts + 'kf-box-hold') && !existsSync(scripts + 'kf-ci-hold.sh'));
});
// ---- the environment the Worker creates ------------------------------------------------
for (const [name, overrides, hasPair] of [
  ['with the Access pair', access, true],
  ['without the Access pair', {}, false],
])
  test('p7 the Worker-created environment passes check-env ' + name, async () => {
    const box = await rented(overrides);
    try {
      assert.equal('KF_ACCESS_ID' in box.env, hasPair);
      const out = box.dir + '/env';
      const r = await run(
        'python3',
        ['-I', scripts + 'vast_report.py', 'check-env', '--write', out],
        { ...baseEnv(), ...box.env },
        box.dir,
      );
      assert.equal(r.status, 0, r.err);
      const written = await readFile(out, 'utf8');
      assert.match(written, /^export KF_PROTOCOL=7$/m);
      assert.equal(written.includes('KF_ACCESS_ID'), hasPair);
      assert.equal(written.includes(access.BOX_ACCESS_SECRET), hasPair);
    } finally {
      await box.close();
    }
  });
for (const [name, patch] of [
  ['protocol 6', { KF_PROTOCOL: '6' }],
  ['protocol 5', { KF_PROTOCOL: '5' }],
  ['v1 token', { KF_RUN_TOKEN: 'v1.123.1.1800001200.' + 'A'.repeat(43) }],
])
  test('p7 check-env refuses an environment that is not protocol 7: ' + name, async () => {
    const box = await rented();
    try {
      const r = await run(
        'python3',
        ['-I', scripts + 'vast_report.py', 'check-env', '--write', box.dir + '/env'],
        { ...baseEnv(), ...box.env, ...patch },
        box.dir,
      );
      assert.notEqual(r.status, 0);
      assert.ok(!existsSync(box.dir + '/env'));
    } finally {
      await box.close();
    }
  });
// ---- reporting end to end --------------------------------------------------------------
for (const [name, overrides, drop, expected] of [
  ['both halves', access, [], true],
  ['no pair', {}, [], false],
  ['id only', access, ['KF_ACCESS_SECRET'], false],
  ['secret only', access, ['KF_ACCESS_ID'], false],
])
  test('p7 onstart run is accepted by the Worker, Access headers ' + name, async () => {
    const box = await rented(overrides);
    for (const key of drop) delete box.env[key];
    try {
      const r = await runOnstart(box, PASS_PAYLOAD);
      assert.equal(r.status, 0, r.out + r.err);
      const record = await box.record();
      assert.equal(record.verdict, 'PASS');
      assert.equal(record.result.hardware.compute_cap, 890);
      assert.equal(record.result.suites.length, sampleResult().suites.length);
      // every request: the run capability, and the Access pair if and only if both were set
      assert.ok(box.seen.length >= 2);
      for (const request of box.seen) {
        assert.equal(request.headers.authorization, 'Bearer ' + box.env.KF_RUN_TOKEN);
        assert.equal(request.headers['cf-access-client-id'] !== undefined, expected);
        assert.equal(request.headers['cf-access-client-secret'] !== undefined, expected);
        if (expected) {
          assert.equal(request.headers['cf-access-client-id'], access.BOX_ACCESS_ID);
          assert.equal(request.headers['cf-access-client-secret'], access.BOX_ACCESS_SECRET);
        }
      }
      // numbered one by one, no gaps, kinds the Worker knows
      const posted = box.posted();
      assert.deepEqual(
        posted.map((p) => p.seq),
        posted.map((_, i) => i + 1),
      );
      assert.ok(posted.every((p) => ['progress', 'log', 'result'].includes(p.kind)));
      assert.equal(posted.at(-1).kind, 'result');
      assert.deepEqual(Object.keys(posted.at(-1).result).sort(), [
        'hardware',
        'nonce',
        'runner_nonce',
        'sha',
        'suites',
        'verdict',
      ]);
      assert.ok(
        posted.every((p) =>
          Object.keys(p).every((k) => ['seq', 'kind', 'text', 'result'].includes(k)),
        ),
      );
      // The payload cannot see the capability, the endpoint or the Access pair.
      const dump = await readFile(box.dir + '/dump', 'utf8');
      for (const secret of [box.env.KF_RUN_TOKEN, box.prefix, access.BOX_ACCESS_SECRET])
        assert.ok(!dump.includes(secret));
      for (const name of ['KF_RUN_TOKEN', 'KF_BROKER_URL', 'KF_RUN_ID', 'KF_RUNNER_NONCE'])
        assert.ok(!dump.includes(name + '='));
      assert.ok(!dump.includes('KF_ACCESS'));
      // No secret reaches a posted line or the box output.
      const everything = box.seen.map((r) => r.body).join('') + r.out + r.err;
      for (const secret of [
        box.env.KF_RUN_TOKEN,
        box.prefix,
        access.BOX_ACCESS_SECRET,
        access.BOX_ACCESS_ID,
      ])
        assert.ok(!everything.includes(secret), 'secret in output');
    } finally {
      await box.close();
    }
  });
test('p7 the bound result matches the Worker record and local diagnostics stay local', async () => {
  const box = await rented();
  try {
    const doc = passDoc({ diagnostics: [] });
    const r = await runOnstart(
      box,
      `echo '${JSON.stringify(doc)}' > "$KF_CI_WORKSPACE/kf-result.json"\n`,
    );
    assert.equal(r.status, 0, r.out + r.err);
    const wire = resultOf(box);
    const record = await box.record();
    assert.equal(wire.sha, record.sha);
    assert.equal(wire.nonce, record.nonce);
    assert.equal(wire.runner_nonce, record.runnerNonce);
    assert.ok(!('diagnostics' in wire));
  } finally {
    await box.close();
  }
});
test('p7 a payload that tries to bind another run is refused, not forwarded', async () => {
  const box = await rented();
  try {
    const doc = passDoc({ sha: 'c'.repeat(40) });
    const r = await runOnstart(
      box,
      `echo '${JSON.stringify(doc)}' > "$KF_CI_WORKSPACE/kf-result.json"\n`,
    );
    assert.notEqual(r.status, 0);
    const record = await box.record();
    // The reporter turned the conflicting file into its own conservative FAIL; the Worker took it.
    assert.equal(record.verdict, 'FAIL');
    assert.equal(resultOf(box).sha, record.sha);
  } finally {
    await box.close();
  }
});
for (const [name, payload, env, row, exit] of [
  ['exit code', PASS_PAYLOAD + 'exit 3\n', {}, 'app_exit', 3],
  ['absent payload', undefined, {}, 'payload_missing', 127],
  ['malformed contract', PASS_PAYLOAD, { KF_GRACE_FAIL: 'soon' }, 'env_contract', 2],
])
  test('p7 a failing box still gives the Worker an acceptable FAIL: ' + name, async () => {
    const box = await rented(access);
    try {
      const r = await runOnstart(box, payload, env);
      for (const request of box.reports()) {
        assert.equal(request.headers['cf-access-client-id'], access.BOX_ACCESS_ID);
        assert.equal(request.headers['cf-access-client-secret'], access.BOX_ACCESS_SECRET);
      }
      assert.notEqual(r.status, 0, 'a failing run exits non-zero');
      assert.equal(r.status, exit, r.out + r.err);
      const record = await box.record();
      assert.equal(record.verdict, 'FAIL');
      assert.ok(
        record.result.suites.some((s) => s.name === row && s.fail > 0),
        row,
      );
    } finally {
      await box.close();
    }
  });
test('p7 a chatty payload is relayed within the Worker caps, every post accepted', async () => {
  const box = await rented();
  try {
    const noisy = 'for i in $(seq 1 120); do head -c 3900 /dev/zero | tr "\\0" "x"; echo; done\n';
    const r = await runOnstart(box, noisy + PASS_PAYLOAD);
    assert.equal(r.status, 0, r.out + r.err);
    const posts = box.reports();
    assert.ok(posts.length > 10, 'many posts');
    assert.ok(
      posts.every((q) => q.status === 200),
      'all answered 200',
    );
    const sizes = posts.map((q) => Buffer.byteLength(q.body));
    assert.ok(Math.max(...sizes) < 65536, 'below the Worker body cap');
    const record = await box.record();
    assert.equal(record.verdict, 'PASS');
    assert.ok(record.reportBytes < DEFAULTS.reportBytes);
    assert.ok(record.reportCount <= DEFAULTS.reportCount);
    assert.ok(record.fullBytes > 1000, 'the relayed text reached the Worker log');
  } finally {
    await box.close();
  }
});
test('p7 the soft deadline is enforced and the Worker gets a timeout FAIL', async () => {
  const box = await rented();
  try {
    const soft = String(Math.floor(Date.now() / 1000) + 6);
    const r = await runOnstart(box, 'sleep 120\n', { KF_SOFT_DEADLINE: soft });
    assert.notEqual(r.status, 0);
    const record = await box.record();
    assert.equal(record.verdict, 'FAIL');
    assert.ok(record.result.suites.some((s) => s.name === 'timeout'));
  } finally {
    await box.close();
  }
});
test('p7 the harness is live: a wrong token is refused by the real Worker', async () => {
  const box = await rented();
  try {
    const wrong = callbackToken(await box.record(), 'another-secret-of-the-right-shape-0123456789');
    const r = await runOnstart(box, PASS_PAYLOAD, { KF_RUN_TOKEN: wrong });
    assert.notEqual(r.status, 0);
    assert.equal((await box.record()).verdict, null);
    assert.ok(
      box.seen.length >= 1 && box.seen.every((q) => q.headers.authorization.includes(wrong)),
    );
  } finally {
    await box.close();
  }
});
// ---- the box helpers -------------------------------------------------------------------
async function envFile(box, patch = {}) {
  const r = await run(
    'python3',
    ['-I', scripts + 'vast_report.py', 'check-env', '--write', box.dir + '/env'],
    { ...baseEnv(), ...box.env, ...patch },
    box.dir,
  );
  assert.equal(r.status, 0, r.err);
  return box.dir + '/env';
}
for (const [name, overrides, expected] of [
  ['with the pair', access, true],
  ['without', {}, false],
])
  test('p7 kf-box-kill ends the run at the Worker ' + name, async () => {
    const box = await rented(overrides);
    try {
      const file = await envFile(box);
      const r = await run(
        'sh',
        [scripts + 'kf-box-kill', 'testing the kill'],
        { ...baseEnv(), KF_CI_ENV_FILE: file },
        box.dir,
      );
      assert.equal(r.status, 0, r.err);
      const kills = box.seen.filter((q) => q.path === '/' + box.prefix + '/kill/' + RUN);
      assert.equal(kills.length, 1);
      assert.equal(kills[0].method, 'POST');
      assert.equal(kills[0].headers.authorization, 'Bearer ' + box.env.KF_RUN_TOKEN);
      assert.equal(kills[0].headers['cf-access-client-id'] !== undefined, expected);
      assert.equal(kills[0].headers['cf-access-client-secret'] !== undefined, expected);
      const record = await box.record();
      assert.equal(record.boxKilled, true);
      assert.ok(record.ended !== null || record.closed);
      assert.deepEqual(box.h.cloud.deletes, ['900']);
    } finally {
      await box.close();
    }
  });
test('p7 kf-box-kill after the run has ended is idempotent at the Worker', async () => {
  const box = await rented();
  try {
    const file = await envFile(box);
    const kill = () =>
      run(
        'sh',
        [scripts + 'kf-box-kill', 'twice'],
        { ...baseEnv(), KF_CI_ENV_FILE: file },
        box.dir,
      );
    assert.equal((await kill()).status, 0);
    assert.equal((await kill()).status, 0);
  } finally {
    await box.close();
  }
});
const signed =
  'https://release-assets.githubusercontent.com/github-production-release-asset/' +
  '123/blob?sig=private-short&se=2026-10-23';
for (const [name, overrides, expected] of [
  ['with the pair', access, true],
  ['without', {}, false],
])
  test('p7 kf-box-blob-url mints a URL through the Worker ' + name, async () => {
    const box = await rented({
      BLOB_REPO: 'owner/private-store',
      BLOB_ALLOWED_TAG_PREFIXES: '["dummy-blobs-"]',
      BLOB_GITHUB_TOKEN: 'mock-blob-read-secret',
      ...overrides,
    });
    try {
      const base = box.h.cloud.fetch;
      box.h.cloud.fetch = async (url, init = {}) => {
        if (!String(url).includes('/repos/owner/private-store/')) return base(url, init);
        if (String(url).includes('/releases/tags/'))
          return Response.json({ assets: [{ id: 17, name: 'blob-a.txt', size: 49 }] });
        return {
          status: 302,
          redirected: false,
          headers: new Headers({ Location: signed }),
          body: { cancel: async () => {} },
        };
      };
      box.h.restart();
      const file = await envFile(box);
      const r = await run(
        'sh',
        [scripts + 'kf-box-blob-url', 'dummy-blobs-20261024', 'blob-a.txt'],
        { ...baseEnv(), KF_CI_ENV_FILE: file },
        box.dir,
      );
      assert.equal(r.status, 0, r.err);
      assert.deepEqual(JSON.parse(r.out), { url: signed, size: 49, name: 'blob-a.txt' });
      const mint = box.seen.find((q) => q.path.includes('/blob-url/'));
      assert.equal(mint.method, 'GET');
      assert.equal(mint.path, `/${box.prefix}/blob-url/${RUN}/dummy-blobs-20261024/blob-a.txt`);
      assert.equal(mint.headers.authorization, 'Bearer ' + box.env.KF_RUN_TOKEN);
      assert.equal(mint.headers['cf-access-client-id'] !== undefined, expected);
      assert.equal(mint.headers['cf-access-client-secret'] !== undefined, expected);
      assert.ok(!r.err.includes(box.env.KF_RUN_TOKEN) && !r.err.includes(access.BOX_ACCESS_SECRET));
    } finally {
      await box.close();
    }
  });
