import test from 'node:test';
import assert from 'node:assert/strict';
import './helpers.js';
import { createGateway } from '../src/index.js';
import { env } from './helpers.js';
import {
  CANONICAL_REPO,
  COMMANDS_MAX_MS,
  COMMIT_SKIP,
  MAIN_REFS,
  explainHosts,
  isGitOnly,
  REPO_ALLOWLIST,
  WIRE_STAGES,
  checkKnownArtifact,
  commandsOpen,
  customHostResultCountable,
  dieOf,
  hexEqual,
  isBorrowed,
  seedAllowed,
  seedFor,
  validateCommandsUntil,
  validateDaemonFrame,
  validateKnownArtifacts,
  validateSeedRef,
  validateServerFrame,
  HostError,
  LIVENESS_MS,
  PROBE_FILE,
  SEED_PROBE_CONTENT,
  boxTokenMatches,
  hashBoxToken,
  hostRejection,
  hostsFor,
  inWindow,
  mintBoxToken,
  probeSeeder,
  MAPPERS,
  probeHostnames,
  seedFallbackUrl,
  publicIp,
  publicIpv4,
  sanitizeCapabilities,
  validateAssignment,
  validateCommand,
  validateHostConfig,
} from '../src/hosts.js';

// FALSIFIER (whole file): a borrowed offer can carry anything but repo/commit/tree_key/allowlisted
// stages; commands reach a non-owned box; a disabled, disconnected, busy, out-of-window or
// under-capable box is selected; a custom host is ranked after another; or the seeder probe contacts
// an address other than the one passed in, follows a redirect or accepts other content.
const refused = (fn, code) =>
  assert.throws(fn, (e) => e instanceof HostError && e.code === code, code);

const goodConfig = {
  enabled: true,
  profile: 'owned',
  tiers: ['middle', 'small'],
  seeder: { enabled: true, remote_ip: '93.184.216.34', external_port: 40022, inner_port: 8080 },
  window: null,
  opportunistic: false,
  notes: 'rack 2',
};

test('config: valid body is normalised, tiers sorted', () => {
  const c = validateHostConfig(goodConfig);
  assert.deepEqual(c.tiers, ['middle', 'small']);
  assert.equal(c.seeder.remote_ip, '93.184.216.34');
  assert.equal(c.window, null);
});
for (const [name, patch, code] of [
  ['unknown key', { extra: 1 }, 'CONFIG_KEYS'],
  ['profile self-declared nonsense', { profile: 'root' }, 'CONFIG_PROFILE'],
  ['enabled not boolean', { enabled: 'yes' }, 'CONFIG_ENABLED'],
  ['tier unknown', { tiers: ['huge'] }, 'CONFIG_TIERS'],
  ['tier duplicate', { tiers: ['small', 'small'] }, 'CONFIG_TIERS'],
  [
    'seeder private ip',
    { seeder: { ...goodConfig.seeder, remote_ip: '10.0.0.5' } },
    'CONFIG_SEEDER_IP',
  ],
  [
    'seeder loopback',
    { seeder: { ...goodConfig.seeder, remote_ip: '127.0.0.1' } },
    'CONFIG_SEEDER_IP',
  ],
  [
    'seeder hostname',
    { seeder: { ...goodConfig.seeder, remote_ip: 'evil.example' } },
    'CONFIG_SEEDER_IP',
  ],
  ['seeder port 0', { seeder: { ...goodConfig.seeder, external_port: 0 } }, 'CONFIG_SEEDER_PORT'],
  [
    'seeder hostname is not configurable',
    { seeder: { ...goodConfig.seeder, hostname: 'seed.example.org' } },
    'CONFIG_SEEDER',
  ],
  ['seeder extra key', { seeder: { ...goodConfig.seeder, url: 'http://x' } }, 'CONFIG_SEEDER'],
  ['window equal', { window: { start: '01:00', end: '01:00' } }, 'CONFIG_WINDOW'],
  ['window format', { window: { start: '1:00', end: '02:00' } }, 'CONFIG_WINDOW'],
  ['notes long', { notes: 'x'.repeat(501) }, 'CONFIG_NOTES'],
])
  test('config refuses ' + name, () =>
    refused(() => validateHostConfig({ ...goodConfig, ...patch }), code),
  );
test('config: a disabled seeder needs no address', () => {
  assert.deepEqual(validateHostConfig({ ...goodConfig, seeder: { enabled: false } }).seeder, {
    enabled: false,
  });
});

test('publicIp', () => {
  for (const ok of ['93.184.216.34', '8.8.8.8', '2001:db8::1']) assert.ok(publicIp(ok), ok);
  for (const no of [
    '0.0.0.0',
    '10.1.1.1',
    '100.64.0.1',
    '127.0.0.1',
    '169.254.169.254',
    '172.16.0.1',
    '172.31.255.255',
    '192.168.1.1',
    '224.0.0.1',
    '256.1.1.1',
    '01.2.3.4',
    '::',
    '::1',
    'fe80::1',
    'fd00::1',
    'ff02::1',
    '::ffff:10.0.0.1',
    'example.com',
    '',
    5,
    null,
  ])
    assert.equal(publicIp(no), null, String(no));
  assert.ok(publicIp('172.32.0.1'));
});

test('window: normal, wrapping, always', () => {
  const at = (h, m) => Date.UTC(2026, 10, 12, h, m);
  assert.ok(inWindow(null, at(3, 0)));
  const day = { start: '08:00', end: '18:00' };
  assert.ok(inWindow(day, at(8, 0)) && inWindow(day, at(17, 59)));
  assert.ok(!inWindow(day, at(18, 0)) && !inWindow(day, at(7, 59)));
  const night = { start: '22:00', end: '06:00' };
  assert.ok(inWindow(night, at(23, 0)) && inWindow(night, at(5, 59)));
  assert.ok(!inWindow(night, at(12, 0)));
});

const assignment = {
  repo: CANONICAL_REPO,
  commit: 'a'.repeat(40),
  tree_key: 'b'.repeat(64),
  stages: ['windows_gate'],
};
test('borrowed assignment: exactly repo, commit, tree_key, allowlisted stages', () => {
  assert.deepEqual(validateAssignment('borrowed', assignment), assignment);
  assert.ok(WIRE_STAGES.includes('windows_gate'));
  for (const extra of [
    { env: { KF_X: '1' } },
    { requirements: {} },
    { script: 'rm -rf /' },
    { cmd: 'x' },
    { url: 'http://x' },
  ])
    refused(
      () => validateAssignment('borrowed', { ...assignment, ...extra }),
      'OFFER_ASSIGNMENT_KEYS',
    );
  for (const stages of [['rm -rf'], ['not_a_stage'], 'gates', [1]])
    refused(() => validateAssignment('borrowed', { ...assignment, stages }), 'OFFER_STAGES');
  refused(() => validateAssignment('borrowed', { ...assignment, commit: 'HEAD' }), 'OFFER_COMMIT');
  refused(() => validateAssignment('borrowed', { ...assignment, repo: '../x' }), 'OFFER_REPO');
  refused(() => validateAssignment('borrowed', { ...assignment, tree_key: '' }), 'OFFER_TREE_KEY');
});
test('owned and container assignments may carry bounded env and requirements', () => {
  for (const p of ['owned', 'container']) {
    const a = validateAssignment(p, {
      ...assignment,
      env: { KF_DEVICE: 'kf3' },
      requirements: { tier: 'small' },
    });
    assert.deepEqual(a.env, { KF_DEVICE: 'kf3' });
    refused(() => validateAssignment(p, { ...assignment, env: { PATH: '/x' } }), 'OFFER_ENV');
  }
  refused(() => validateAssignment('root', assignment), 'OFFER_PROFILE');
});

test('commands: owned only, exactly one of argv or shell', () => {
  assert.deepEqual(validateCommand('owned', { argv: ['ls', '-l'] }), { argv: ['ls', '-l'] });
  assert.deepEqual(validateCommand('owned', { shell: 'uptime' }), { shell: 'uptime' });
  for (const p of ['container', 'borrowed']) {
    try {
      validateCommand(p, { argv: ['ls'] });
      assert.fail();
    } catch (e) {
      assert.equal(e.code, 'CMD_NOT_ALLOWED');
      assert.equal(e.status, 403);
    }
  }
  refused(() => validateCommand('owned', { argv: ['a'], shell: 'b' }), 'CMD_SHAPE');
  refused(() => validateCommand('owned', {}), 'CMD_SHAPE');
  refused(() => validateCommand('owned', { argv: [] }), 'CMD_SHAPE');
  refused(() => validateCommand('owned', { argv: [1] }), 'CMD_SHAPE');
  refused(() => validateCommand('owned', { shell: 'x', extra: 1 }), 'CMD_SHAPE');
});

test('capabilities: unknown and ill-typed fields are dropped', () => {
  const caps = sanitizeCapabilities({
    gpu_name: 'RTX 4070',
    compute_cap: '8.9',
    kvm: 'yes',
    admin: true,
    module_flavor: 'weird',
    vram_gb: -1,
    cores: 16,
  });
  assert.deepEqual(caps, { gpu_name: 'RTX 4070', compute_cap: '8.9', cores: 16 });
  assert.deepEqual(sanitizeCapabilities(null), {});
});

// ---- selection ---------------------------------------------------------------------------------
const NOW = Date.UTC(2026, 10, 12, 12, 0);
const snap = (id, patch = {}, cfg = {}, caps = {}) => ({
  box_id: id,
  config: { ...validateHostConfig({ ...goodConfig, tiers: ['small', 'middle', 'large'] }), ...cfg },
  capabilities: {
    compute_cap: '8.9',
    die: 'AD104',
    vram_gb: 12,
    driver: '580.1',
    kvm: true,
    display: true,
    can_swap_driver: true,
    ...caps,
  },
  connected: true,
  state: 'idle',
  last_heartbeat: NOW - 1000,
  offer_pending: false,
  ...patch,
});
const req = { tier: 'middle', family: 'ada' };
test('hostsFor: qualifying box is returned', () => {
  assert.deepEqual(hostsFor(req, [snap('a')], NOW), [
    { box_id: 'a', profile: 'owned', opportunistic: false, hourly_price_usd: null },
  ]);
});
for (const [name, snapshot, reason] of [
  ['disabled', snap('a', {}, { enabled: false }), 'DISABLED'],
  ['not connected', snap('a', { connected: false }), 'NOT_CONNECTED'],
  ['stale heartbeat', snap('a', { last_heartbeat: NOW - LIVENESS_MS }), 'NOT_CONNECTED'],
  ['busy', snap('a', { state: 'busy' }), 'BUSY'],
  ['offer pending', snap('a', { offer_pending: true }), 'BUSY'],
  ['tier not enabled', snap('a', {}, { tiers: ['small'] }), 'TIER'],
  ['outside window', snap('a', {}, { window: { start: '01:00', end: '02:00' } }), 'WINDOW'],
  ['wrong family', snap('a', {}, {}, { compute_cap: '8.0' }), 'FAMILY'],
  ['no capabilities', snap('a', { capabilities: {} }), 'FAMILY'],
])
  test('hostsFor rejects ' + name, () => {
    assert.equal(hostRejection(req, snapshot, NOW), reason);
    assert.deepEqual(hostsFor(req, [snapshot], NOW), []);
  });
test('hostsFor: unknown tier is never selected', () => {
  assert.equal(hostRejection({ tier: 'huge' }, snap('a'), NOW), 'REQ_TIER');
  assert.equal(hostRejection({ tier: 'largest' }, snap('a'), NOW), null);
});
test('hostsFor: capability requirements', () => {
  const push = { event_name: 'push', ref: 'refs/heads/next' };
  const r = (extra, s = snap('a')) => hostRejection({ ...req, run: push, ...extra }, s, NOW);
  assert.equal(r({ min_vram_gb: 16 }), 'VRAM');
  assert.equal(r({ min_vram_gb: 12 }), null);
  assert.equal(r({ kvm: true }, snap('a', {}, {}, { kvm: false })), 'KVM');
  assert.equal(r({ display: true }, snap('a', {}, {}, { display: false })), 'DISPLAY');
  assert.equal(r({ die: 'AD104' }), null);
  assert.equal(r({ die: 'AD104' }, snap('a', {}, {}, { die: 'AD106' })), 'DIE');
  assert.equal(r({ family: undefined, die: 'GA102' }), 'FAMILY');
  // Driver: a mismatch is fine on an owned box that can swap, never on borrowed or container.
  assert.equal(r({ driver: '595.0' }), null);
  assert.equal(r({ needs_driver_swap: true }), null);
  for (const profile of ['container', 'borrowed']) {
    const s = snap('a', {}, { profile });
    assert.equal(r({ driver: '595.0' }, s), 'DRIVER');
    assert.equal(r({ needs_driver_swap: true }, s), 'DRIVER_SWAP');
    assert.equal(r({ driver: '580.1' }, s), null);
  }
  assert.equal(
    r({ needs_driver_swap: true }, snap('a', {}, {}, { can_swap_driver: false })),
    'DRIVER_SWAP',
  );
});
test('hostsFor: non-opportunistic first, then by id; input untouched', () => {
  const list = [
    snap('c', {}, { opportunistic: true }),
    snap('b'),
    snap('a', {}, { opportunistic: true }),
    snap('d'),
    snap('z', { connected: false }),
  ];
  const copy = structuredClone(list);
  assert.deepEqual(
    hostsFor(req, list, NOW).map((x) => x.box_id),
    ['b', 'd', 'a', 'c'],
  );
  assert.deepEqual(list, copy);
  assert.deepEqual(hostsFor(req, [], NOW), []);
});

// ---- token -------------------------------------------------------------------------------------
test('box token: minted shape, hashed compare, wrong and malformed refused', async () => {
  const token = mintBoxToken();
  assert.match(token, /^kfh_[A-Za-z0-9_-]{43}$/);
  assert.notEqual(token, mintBoxToken());
  const hash = await hashBoxToken(token);
  assert.match(hash, /^[0-9a-f]{64}$/);
  assert.ok(!hash.includes(token));
  assert.equal(await boxTokenMatches(hash, token), true);
  assert.equal(await boxTokenMatches(hash, mintBoxToken()), false);
  assert.equal(await boxTokenMatches(hash, token + 'x'), false);
  assert.equal(await boxTokenMatches(hash, undefined), false);
  assert.equal(await boxTokenMatches(undefined, token), false);
  assert.equal(await boxTokenMatches(token, token), false, 'a raw token is not a stored hash');
});

// ---- probe -------------------------------------------------------------------------------------
const TOKEN = 'tokn1234';
const answer =
  (body, init = {}) =>
  async () =>
    new Response(body, init);
const probe = (fetcher, extra = {}) =>
  probeSeeder({ fetcher, ip: '93.184.216.34', port: 40022, token: TOKEN, timeoutMs: 40, ...extra });
test('probe success contacts exactly the configured address, once', async () => {
  const urls = [];
  const options = [];
  const r = await probe(async (url, init) => {
    urls.push(url);
    options.push(init);
    return new Response(SEED_PROBE_CONTENT);
  });
  assert.deepEqual(r, { ok: true, hostname: '93-184-216-34.sslip.io' });
  assert.deepEqual(urls, ['http://93-184-216-34.sslip.io:40022/' + TOKEN + '/' + PROBE_FILE]);
  assert.equal(options[0].redirect, 'manual');
});
test('probe refuses an IPv6 literal (not supported for now)', async () => {
  assert.equal(
    (await probe(async () => new Response(SEED_PROBE_CONTENT), { ip: '2001:db8::1' })).reason,
    'BAD_ADDRESS',
  );
});
test('probe: wrong content fails every name (one attempt each), prefix or suffix differences too', async () => {
  for (const body of ['nope', SEED_PROBE_CONTENT + 'x', SEED_PROBE_CONTENT.trim(), '']) {
    let calls = 0;
    const r = await probe(async () => (calls++, new Response(body)));
    assert.deepEqual(r, { ok: false, reason: 'WRONG_CONTENT' });
    assert.equal(calls, 2);
  }
});
test('probe: an oversized body is wrong content', async () => {
  const r = await probe(answer('x'.repeat(5000)));
  assert.equal(r.reason, 'WRONG_CONTENT');
});
test('probe: redirects are refused and not followed (a failure of that name)', async () => {
  let calls = 0;
  const r = await probe(
    async () => (
      calls++,
      new Response(null, { status: 302, headers: { Location: 'http://10.0.0.1/' } })
    ),
  );
  assert.deepEqual(r, { ok: false, reason: 'REDIRECT' });
  assert.equal(calls, 2);
  const o = await probe(async () => ({ type: 'opaqueredirect', status: 0 }));
  assert.equal(o.reason, 'REDIRECT');
});
test('probe: timeout on each name, then false; a fetcher that ignores the signal is bounded too', async () => {
  let calls = 0;
  const hang = () => (calls++, new Promise(() => {}));
  const t0 = Date.now();
  const r = await probe(hang);
  assert.deepEqual(r, { ok: false, reason: 'TIMEOUT' });
  assert.equal(calls, 2);
  assert.ok(Date.now() - t0 < 1000);
  const aborted = await probe(
    (url, { signal }) =>
      new Promise((_, rej) => signal.addEventListener('abort', () => rej(new Error('x')))),
  );
  assert.equal(aborted.reason, 'TIMEOUT');
});
test('probe: the timeout is capped at 5 s even if asked for more', async () => {
  const real = globalThis.setTimeout;
  const seen = [];
  globalThis.setTimeout = (fn, ms, ...a) => (seen.push(ms), real(fn, 0, ...a));
  try {
    await probe(() => new Promise(() => {}), { timeoutMs: 60000 });
  } finally {
    globalThis.setTimeout = real;
  }
  assert.ok(seen.length && seen.every((ms) => ms <= 5000));
});
test('probe: the second name recovers from a failure of the first; two HTTP errors fail', async () => {
  let calls = 0;
  const flaky = async () => {
    if (calls++ === 0) throw new Error('reset');
    return new Response(SEED_PROBE_CONTENT);
  };
  assert.deepEqual(await probe(flaky), { ok: true, hostname: '93-184-216-34.traefik.me' });
  calls = 0;
  const r = await probe(async () => (calls++, new Response('x', { status: 503 })));
  assert.deepEqual(r, { ok: false, reason: 'HTTP_503' });
  assert.equal(calls, 2);
});
test('probe: bad address, port or token never reaches the fetcher', async () => {
  let calls = 0;
  const f = async () => (calls++, new Response(SEED_PROBE_CONTENT));
  for (const [patch, reason] of [
    [{ ip: '10.0.0.1' }, 'BAD_ADDRESS'],
    [{ ip: '127.0.0.1' }, 'BAD_ADDRESS'],
    [{ ip: 'evil.example' }, 'BAD_ADDRESS'],
    [{ ip: '2001:db8::1' }, 'BAD_ADDRESS'],
    [{ ip: '93.184.216.34@evil.example' }, 'BAD_ADDRESS'],
    [{ port: 0 }, 'BAD_PORT'],
    [{ port: 70000 }, 'BAD_PORT'],
    [{ port: '80' }, 'BAD_PORT'],
    [{ token: '../etc/passwd' }, 'BAD_TOKEN'],
    [{ token: 'short' }, 'BAD_TOKEN'],
    [{ token: 'TOKN1234' }, 'BAD_TOKEN'],
    [{ token: 'tokn12345' }, 'BAD_TOKEN'],
    [{ token: TOKEN + '/../x' }, 'BAD_TOKEN'],
  ])
    assert.equal((await probe(f, patch)).reason, reason, JSON.stringify(patch));
  assert.equal(
    (await probeSeeder({ ip: '93.184.216.34', port: 80, token: TOKEN })).reason,
    'NO_FETCHER',
  );
  assert.equal(calls, 0);
});

// ---- gateway route -----------------------------------------------------------------------------
test('gateway forwards /hosts/<id>/ws to that box DO on the internal origin, nothing else', async () => {
  const seen = [];
  const names = [];
  const HOSTS = {
    idFromName: (n) => (names.push(n), n),
    get: () => ({ fetch: async (r) => (seen.push(r), new Response('forwarded')) }),
  };
  const gw = createGateway({
    authorize: async () => assert.fail('no run-token/OIDC check for the host socket'),
  });
  const e = { ...env, RENTING_ENABLED: 'false', HOSTS };
  const mk = (path, init = {}) =>
    gw.fetch(
      new Request('https://broker.example' + path, {
        headers: { Upgrade: 'websocket', Authorization: 'Bearer kfh_' + 'A'.repeat(43) + '' },
        ...init,
      }),
      e,
    );
  const ok = await mk('/hosts/rack-1/ws');
  assert.equal(await ok.text(), 'forwarded');
  assert.deepEqual(names, ['rack-1']);
  assert.equal(seen[0].url, 'https://internal.invalid/hosts/rack-1/ws');
  assert.equal(seen[0].headers.get('Authorization'), 'Bearer kfh_' + 'A'.repeat(43) + '');
  for (const bad of [
    '/hosts/Rack/ws',
    '/hosts/a/b/ws',
    '/hosts//ws',
    '/hosts/-x/ws',
    '/hosts/rack-1',
    '/host/ws/rack-1',
    '/hosts/rack-1/ws/x',
  ])
    assert.equal((await mk(bad)).status, 404, bad);
  assert.equal((await mk('/hosts/rack-1/ws', { method: 'POST', body: 'x' })).status, 404);
  assert.equal(
    (
      await gw.fetch(new Request('https://broker.example/hosts/rack-1/ws'), {
        ...e,
        HOSTS: undefined,
      })
    ).status,
    404,
  );
  assert.equal(seen.length, 1);
});

// ---- probe hostnames (MEASURED 2026-11-12: Workers cannot fetch a raw IP, hostnames work) ---------
test('probeHostnames: sslip.io then traefik.me, dash form, nothing else', () => {
  assert.deepEqual(probeHostnames('92.67.98.9'), ['92-67-98-9.sslip.io', '92-67-98-9.traefik.me']);
  assert.deepEqual(probeHostnames('92.67.98.9', 'seed.example.org'), probeHostnames('92.67.98.9'));
  assert.deepEqual(
    MAPPERS.map((m) => [m.zone, m.enabled]),
    [
      ['sslip.io', true],
      ['traefik.me', true],
      ['local-ip.sh', false],
    ],
  );
  assert.ok(
    !probeHostnames('92.67.98.9').some((n) =>
      /nip\.io$|local-ip|cloud-ip|xip/.test(n.replace('sslip.io', '')),
    ),
  );
});
test('probeHostnames: only a public dotted-quad; no IPv6, private, CGNAT, loopback, link-local', () => {
  for (const bad of [
    '10.0.0.1',
    '172.16.5.5',
    '192.168.0.1',
    '100.64.0.1',
    '127.0.0.1',
    '169.254.1.1',
    '0.0.0.0',
    '::1',
    '2001:db8::1',
    'example.com',
    '1.2.3',
    '1.2.3.4.5',
    '256.1.1.1',
    '1.2.3.4 ',
    '0x7f.0.0.1',
    '',
    null,
    5,
  ])
    assert.deepEqual(probeHostnames(bad), [], String(bad));
  assert.equal(publicIpv4('2001:db8::1'), null);
});
test('probe uses hostnames, never the raw IP; one attempt per name, in order', async () => {
  const urls = [];
  const r = await probe(async (u) => (urls.push(u), new Response('x', { status: 503 })));
  assert.deepEqual(r, { ok: false, reason: 'HTTP_503' });
  assert.deepEqual(urls, [
    'http://93-184-216-34.sslip.io:40022/' + TOKEN + '/probe.txt',
    'http://93-184-216-34.traefik.me:40022/' + TOKEN + '/probe.txt',
  ]);
  for (const u of urls) assert.doesNotMatch(new URL(u).hostname, /^[0-9.]+$/);
});
test('probe: first name fails, second succeeds => ok with the second name; the first success stops the walk', async () => {
  let n = 0;
  const hosts = [];
  const r = await probe(async (u) => {
    hosts.push(new URL(u).hostname);
    return n++ === 0 ? new Response('no', { status: 522 }) : new Response(SEED_PROBE_CONTENT);
  });
  assert.deepEqual(r, { ok: true, hostname: '93-184-216-34.traefik.me' });
  assert.deepEqual(hosts, ['93-184-216-34.sslip.io', '93-184-216-34.traefik.me']);
  const first = [];
  const ok = await probe(async (u) => (first.push(u), new Response(SEED_PROBE_CONTENT)));
  assert.equal(ok.hostname, '93-184-216-34.sslip.io');
  assert.equal(first.length, 1);
});

test('seedFallbackUrl: the hostname that passed, else the first mapper; never an unknown host', () => {
  const f = (ip, host) => seedFallbackUrl(ip, 40022, TOKEN, 'kf3.tar', host);
  assert.equal(f('93.184.216.34'), 'http://93-184-216-34.sslip.io:40022/' + TOKEN + '/kf3.tar');
  assert.equal(
    f('93.184.216.34', '93-184-216-34.traefik.me'),
    'http://93-184-216-34.traefik.me:40022/' + TOKEN + '/kf3.tar',
  );
  assert.equal(f('93.184.216.34', 'evil.example'), null);
  assert.equal(f('10.0.0.1'), null);
  assert.equal(f('2001:db8::1'), null);
});

// ---- review fixes (2026-11-12) -----------------------------------------------------------------
import { configuration } from '../src/policy.js';
import { RunTokens } from '../src/tokens.js';

test('config: profile defaults to standard; the command window is not a config key', () => {
  const { profile, ...rest } = goodConfig;
  assert.equal(validateHostConfig(rest).profile, 'standard');
  refused(() => validateHostConfig({ ...goodConfig, commands_until: 1 }), 'CONFIG_KEYS');
  for (const p of ['owned', 'container', 'standard', 'borrowed'])
    assert.equal(validateHostConfig({ ...goodConfig, profile: p }).profile, p);
});

test('assignment: the canonical repo URL for EVERY profile, never an arbitrary URL', () => {
  assert.equal(CANONICAL_REPO, 'https://github.com/reindertpelsma/kayfabe');
  assert.deepEqual(REPO_ALLOWLIST, [CANONICAL_REPO]);
  for (const p of ['owned', 'container', 'standard', 'borrowed']) {
    for (const repo of [
      'https://evil.example/kayfabe',
      'http://github.com/reindertpelsma/kayfabe',
      CANONICAL_REPO + '.git',
      CANONICAL_REPO + '/',
      'reindertpelsma/kayfabe',
      'file:///etc',
    ])
      refused(() => validateAssignment(p, { ...assignment, repo }), 'OFFER_REPO');
    assert.equal(validateAssignment(p, assignment).repo, CANONICAL_REPO);
  }
  // The allowlist is configurable (an array of URLs), not widened by the box.
  const other = 'https://github.com/example/fork';
  assert.equal(validateAssignment('owned', { ...assignment, repo: other }, [other]).repo, other);
  refused(() => validateAssignment('owned', assignment, [other]), 'OFFER_REPO');
});
test('assignment: wire formats are exact (tree key, commit, stage names)', () => {
  for (const tree_key of [
    'B'.repeat(64),
    'b'.repeat(63),
    'b'.repeat(65),
    'kf-tree-v2:' + 'b'.repeat(16),
  ])
    refused(() => validateAssignment('owned', { ...assignment, tree_key }), 'OFFER_TREE_KEY');
  for (const commit of ['A'.repeat(40), 'a'.repeat(39), 'a'.repeat(64)])
    refused(() => validateAssignment('owned', { ...assignment, commit }), 'OFFER_COMMIT');
  assert.deepEqual(WIRE_STAGES, [
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
  for (const p of ['owned', 'standard']) {
    refused(() => validateAssignment(p, { ...assignment, stages: ['rm -rf'] }), 'OFFER_STAGES');
    refused(() => validateAssignment(p, { ...assignment, stages: ['gates'] }), 'OFFER_STAGES');
    assert.deepEqual(
      validateAssignment(p, { ...assignment, stages: [...WIRE_STAGES] }).stages,
      WIRE_STAGES,
    );
  }
});

test('commands: owned AND an admin window of at most 4 h; unset or expired is closed', () => {
  const now = Date.now();
  assert.equal(validateCommandsUntil(null, now), null);
  assert.equal(validateCommandsUntil(now + 1000, now), now + 1000);
  assert.equal(validateCommandsUntil(now + COMMANDS_MAX_MS, now), now + COMMANDS_MAX_MS);
  for (const bad of [now, now - 1, now + COMMANDS_MAX_MS + 1, '1', 1.5, undefined, NaN])
    refused(() => validateCommandsUntil(bad, now), 'COMMANDS_UNTIL');
  const owned = { profile: 'owned', commands_until: now + 1000 };
  assert.equal(commandsOpen(owned, now), true);
  assert.equal(commandsOpen(owned, now + 1000), false, 'expired');
  assert.equal(commandsOpen({ profile: 'owned' }, now), false, 'unset');
  assert.equal(commandsOpen({ profile: 'owned', commands_until: null }, now), false);
  for (const profile of ['container', 'standard', 'borrowed'])
    assert.equal(commandsOpen({ profile, commands_until: now + 1000 }, now), false, profile);
});

test('die: derived by the Worker from name and compute capability, fail closed', () => {
  const cfg = configuration(env);
  assert.equal(dieOf({ gpu_name: 'NVIDIA GeForce RTX 4070', compute_cap: '8.9' }, cfg), 'AD104');
  assert.equal(dieOf({ gpu_name: 'NVIDIA GeForce RTX 3080', compute_cap: '8.6' }, cfg), 'GA102');
  assert.equal(dieOf({ gpu_name: 'NVIDIA GeForce RTX 9999', compute_cap: '8.9' }, cfg), 'CC_89');
  assert.equal(dieOf({ gpu_name: 'RTX 4070' }, cfg), undefined);
  assert.equal(dieOf({ compute_cap: '8.9' }, cfg), undefined);
  assert.equal(dieOf({ gpu_name: 'RTX 4070', compute_cap: '89' }, cfg), undefined);
  assert.equal(dieOf(null, cfg), undefined);
  // The box cannot claim a die: it is not a capability field.
  assert.equal('die' in sanitizeCapabilities({ die: 'AD104', gpu_name: 'x' }), false);
  assert.deepEqual(sanitizeCapabilities({ pci_device_id: '2786' }), { pci_device_id: '2786' });
  assert.deepEqual(sanitizeCapabilities({ pci_device_id: '0x2786' }), {});
});
test('selection: a box without a derived die never satisfies a die requirement', () => {
  const noDie = snap('a', { capabilities: { compute_cap: '8.9', vram_gb: 12 } });
  assert.equal(hostRejection({ ...req, die: 'AD104' }, noDie, NOW), 'DIE');
  assert.equal(hostRejection({ tier: 'middle', die: 'AD104' }, noDie, NOW), 'DIE');
  assert.equal(hostRejection(req, noDie, NOW), null, 'a family-only requirement still works');
});

test('seeds: every host trusts every other except that a borrowed host never consumes', () => {
  const record = { url: 'u', sha256: 'a'.repeat(64), size: 1, source: 'seeder_reported' };
  const seeders = ['owned', 'container', 'standard', 'borrowed'];
  for (const seeder of seeders)
    for (const consumer of seeders) {
      const expected = consumer !== 'borrowed';
      assert.equal(seedAllowed(consumer), expected, seeder + '->' + consumer);
      assert.equal(seedFor({ ...record, seeder_profile: seeder }, consumer) !== null, expected);
    }
  assert.equal(seedFor(null, 'owned'), null);
  assert.ok(isBorrowed('borrowed') && !isBorrowed('standard'));
});
test('seeds: an independently known hash and size must match; none known = seeder_reported', () => {
  const art = { name: 'kf3.tar', sha256: 'd'.repeat(64), size: 10 };
  assert.equal(checkKnownArtifact(undefined, art), 'seeder_reported');
  assert.equal(checkKnownArtifact({}, art), 'seeder_reported');
  assert.equal(
    checkKnownArtifact({ 'kf3.tar': { sha256: art.sha256, size: 10 } }, art),
    'worker_pinned',
  );
  refused(
    () => checkKnownArtifact({ 'kf3.tar': { sha256: 'e'.repeat(64), size: 10 } }, art),
    'HASH_MISMATCH',
  );
  refused(
    () => checkKnownArtifact({ 'kf3.tar': { sha256: art.sha256, size: 11 } }, art),
    'HASH_MISMATCH',
  );
  refused(
    () => checkKnownArtifact({ 'other.tar': { sha256: art.sha256, size: 10 } }, art),
    'ARTIFACT_UNKNOWN',
  );
  assert.equal(validateKnownArtifacts(undefined), undefined);
  refused(
    () => validateKnownArtifacts({ 'a b': { sha256: 'd'.repeat(64), size: 1 } }),
    'OFFER_ARTIFACTS',
  );
  refused(
    () => validateKnownArtifacts({ a: { sha256: 'D'.repeat(64), size: 1 } }),
    'OFFER_ARTIFACTS',
  );
  refused(
    () => validateKnownArtifacts({ a: { sha256: 'd'.repeat(64), size: 0 } }),
    'OFFER_ARTIFACTS',
  );
});

test('a custom-host result counts only for an enabled tier of that box and profile', () => {
  const config = {
    box_id: 'rack-1',
    profile: 'standard',
    enabled: true,
    tiers: ['small', 'middle'],
  };
  const claims = { box: 'rack-1', hp: 'standard' };
  assert.equal(customHostResultCountable(claims, config, 'small'), true);
  assert.equal(customHostResultCountable(claims, config, 'middle'), true);
  assert.equal(customHostResultCountable(claims, config, 'large'), false);
  assert.equal(customHostResultCountable(claims, config, 'largest'), false);
  assert.equal(customHostResultCountable(claims, { ...config, enabled: false }, 'small'), false);
  assert.equal(customHostResultCountable(claims, { ...config, box_id: 'rack-2' }, 'small'), false);
  assert.equal(customHostResultCountable(claims, { ...config, profile: 'owned' }, 'small'), false);
  assert.equal(customHostResultCountable({}, config, 'small'), false, 'a Vast token has no box');
  assert.equal(customHostResultCountable(claims, config, 'weird'), false);
});
test('run token: box and profile claims, both or neither', async () => {
  const tokens = new RunTokens(env.RUN_TOKEN_SECRET);
  const base = {
    run: '123',
    attempt: '1',
    sha: 'a'.repeat(40),
    nonce: 'c'.repeat(32),
    key: '123-1',
    label: 'kf-ci-123-1',
    reportExpiry: Math.floor(Date.now() / 1000) + 600,
  };
  const withHost = await tokens.verifyStateless(
    await tokens.issue({ ...base, box: 'rack-1', hp: 'standard' }),
  );
  assert.equal(withHost.box, 'rack-1');
  assert.equal(withHost.hp, 'standard');
  const plain = await tokens.verifyStateless(await tokens.issue(base));
  assert.equal(plain.box, undefined);
  for (const extra of [
    { box: 'rack-1' },
    { hp: 'standard' },
    { box: 'Rack', hp: 'standard' },
    { box: 'rack-1', hp: 'root' },
  ])
    await assert.rejects(tokens.issue({ ...base, ...extra }));
});
test('hexEqual is exact and length-safe', () => {
  assert.ok(hexEqual('ab12', 'ab12'));
  assert.ok(!hexEqual('ab12', 'ab13'));
  assert.ok(!hexEqual('ab12', 'ab1'));
  assert.ok(!hexEqual(undefined, 'ab'));
});

test('selection: cheaper hourly price first; unknown price after known; price never qualifies a box', () => {
  const priced = (id, price, cfg = {}) => snap(id, {}, { hourly_price_usd: price, ...cfg });
  const list = [
    priced('a', null),
    priced('b', 0.9),
    priced('c', 0.2),
    priced('d', 0),
    priced('e', 0.2),
    priced('f', 0.1, { opportunistic: true }),
    priced('g', 0.05, { enabled: false }),
  ];
  assert.deepEqual(
    hostsFor(req, list, NOW).map((x) => [x.box_id, x.hourly_price_usd]),
    [
      ['d', 0],
      ['c', 0.2],
      ['e', 0.2],
      ['b', 0.9],
      ['a', null],
      ['f', 0.1],
    ],
  );
  refused(() => validateHostConfig({ ...goodConfig, hourly_price_usd: 50.01 }), 'CONFIG_PRICE');
  refused(() => validateHostConfig({ ...goodConfig, hourly_price_usd: -0.01 }), 'CONFIG_PRICE');
  refused(() => validateHostConfig({ ...goodConfig, hourly_price_usd: NaN }), 'CONFIG_PRICE');
  assert.equal(validateHostConfig({ ...goodConfig, hourly_price_usd: 50 }).hourly_price_usd, 50);
  assert.equal(validateHostConfig(goodConfig).hourly_price_usd, null);
});

test('git-only boxes are scheduled only commits the Worker knows are in master or next history', () => {
  const run = (event_name, ref) => ({ ...req, run: { event_name, ref } });
  const git = snap('g', {}, { profile: 'borrowed' });
  const std = snap('s', {}, { profile: 'standard' });
  assert.equal(hostRejection(run('push', 'refs/heads/master'), git, NOW), null);
  assert.equal(hostRejection(run('push', 'refs/heads/next'), git, NOW), null);
  for (const bad of [
    run('pull_request', 'refs/pull/1/merge'),
    run('pull_request', 'refs/heads/next'),
    run('workflow_dispatch', 'refs/heads/master'),
    run('workflow_dispatch', 'refs/heads/feature'),
    run('push', 'refs/heads/feature'),
    run('push', 'refs/tags/v1'),
    run('push', 'refs/heads/master2'),
    req,
    { ...req, run: {} },
    { ...req, run: null },
  ])
    assert.equal(hostRejection(bad, git, NOW), COMMIT_SKIP, JSON.stringify(bad.run));
  // A snapshot that reports enforced.trust_git_only is git-only whatever its profile.
  const enforced = { ...std, enforced: { trust_git_only: true } };
  assert.equal(hostRejection(run('pull_request', 'refs/pull/1/merge'), enforced, NOW), COMMIT_SKIP);
  assert.equal(hostRejection(run('push', 'refs/heads/next'), enforced, NOW), null);
  // Everyone else has no commit restriction, with or without claims.
  for (const s of [
    std,
    snap('o', {}, { profile: 'owned' }),
    snap('c', {}, { profile: 'container' }),
  ])
    for (const r of [
      run('pull_request', 'refs/pull/1/merge'),
      run('workflow_dispatch', 'refs/heads/x'),
      req,
    ])
      assert.equal(hostRejection(r, s, NOW), null);
  assert.equal(isGitOnly(git), true);
  assert.equal(isGitOnly(std), false);
  assert.deepEqual(MAIN_REFS, ['refs/heads/master', 'refs/heads/next']);
  // hostsFor skips the git-only box for a PR and the skip reason is recorded for debug output.
  const pr = run('pull_request', 'refs/pull/1/merge');
  assert.deepEqual(
    hostsFor(pr, [git, std], NOW).map((x) => x.box_id),
    ['s'],
  );
  assert.deepEqual(explainHosts(pr, [git, std], NOW), [
    { box_id: 'g', reason: COMMIT_SKIP },
    { box_id: 's', reason: null },
  ]);
  assert.deepEqual(
    hostsFor(run('push', 'refs/heads/next'), [git, std], NOW).map((x) => x.box_id),
    ['g', 's'],
  );
});

test('publicIp: documentation, 6to4-relay and reserved space are refused', () => {
  for (const ip of [
    '192.0.2.1',
    '198.51.100.7',
    '203.0.113.9',
    '192.88.99.1',
    '240.0.0.1',
    '255.255.255.255',
  ])
    assert.equal(publicIp(ip), null, ip);
  for (const ip of [
    '93.184.216.34',
    '8.8.8.8',
    '192.0.3.1',
    '198.51.101.1',
    '203.0.114.1',
    '192.88.100.1',
  ])
    assert.equal(publicIp(ip), ip, ip);
});

test('assignment: a planner can never supply a seed; the Worker-built reference is validated', () => {
  const seed = {
    url: 'http://93.184.216.34:40022/ab12cd34/kf3.tar',
    fallback_url: 'http://93-184-216-34.sslip.io:40022/ab12cd34/kf3.tar',
    sha256: 'd'.repeat(64),
    size: 1234,
    name: 'kf3.tar',
  };
  for (const p of ['owned', 'container', 'standard', 'borrowed']) {
    refused(() => validateAssignment(p, { ...assignment, seed }), 'OFFER_ASSIGNMENT_KEYS');
  }
  const ok = validateAssignment('standard', { ...assignment, seed }, REPO_ALLOWLIST, {
    allowSeed: true,
  });
  assert.deepEqual(ok.seed, seed);
  assert.equal(
    validateAssignment(
      'standard',
      { ...assignment, seed: { ...seed, fallback_url: null } },
      REPO_ALLOWLIST,
      { allowSeed: true },
    ).seed.fallback_url,
    null,
  );
  // A borrowed consumer is never given a seed, even by the frame check's switch.
  refused(
    () =>
      validateAssignment('borrowed', { ...assignment, seed }, REPO_ALLOWLIST, { allowSeed: true }),
    'OFFER_ASSIGNMENT_KEYS',
  );
  for (const bad of [
    { ...seed, url: 'http://10.0.0.1:40022/ab12cd34/kf3.tar' },
    { ...seed, url: 'https://93.184.216.34:40022/ab12cd34/kf3.tar' },
    { ...seed, url: 'http://example.com:40022/ab12cd34/kf3.tar' },
    { ...seed, url: 'http://93.184.216.34:40022/AB12CD34/kf3.tar' },
    { ...seed, url: 'http://93.184.216.34:40022/ab12cd34/other.tar' },
    { ...seed, fallback_url: 'http://93-184-216-35.sslip.io:40022/ab12cd34/kf3.tar' },
    { ...seed, fallback_url: 'http://evil.example:40022/ab12cd34/kf3.tar' },
    { ...seed, fallback_url: 'http://93-184-216-34.sslip.io:40023/ab12cd34/kf3.tar' },
    { ...seed, sha256: 'x' },
    { ...seed, size: 0 },
    { ...seed, extra: 1 },
  ])
    refused(() => validateSeedRef(bad), 'OFFER_SEED');
});
