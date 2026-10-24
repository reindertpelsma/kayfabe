import { webcrypto, createHmac, timingSafeEqual } from 'node:crypto';
import { useTimingSafeEqual } from '../src/tokens.js';
// Plain Node has no crypto.subtle.timingSafeEqual (a Workers API): install Node's.
useTimingSafeEqual(timingSafeEqual);
import { readFileSync } from 'node:fs';
// The tested commit's reporter, served by the mock contents API (V6 protocol marker).
export const reporterSource = readFileSync(
  new URL('../../../scripts/ci/vast_report.py', import.meta.url),
  'utf8',
);
export const clock = 1800000000000;
export const sha = 'a'.repeat(40);
export const nonce = 'b'.repeat(32);
export const owner = '42';
export const repo = 'reindertpelsma/kayfabe';
export const env = {
  RENTING_ENABLED: 'true',
  OWNER_GITHUB_ID: owner,
  REPOSITORY_GITHUB_ID: '1234567',
  REPOSITORY: repo,
  WORKER_URL: 'https://broker.example',
  APPROVAL_MODE: 'owner_push_or_review',
  RENT_DRAFTS: 'false',
  CREDIT_RESERVE_USD: '8',
  MAX_PRICE_USD_HOUR: '0.5',
  HARD_RUNTIME_MINUTES: '20',
  BOOT_LIMIT_MINUTES: '10',
  DAILY_RENTALS: '4',
  DAILY_SPEND_USD: '3',
  ROLLING_SPEND_USD: '30',
  SHA_GAP_HOURS: '24',
  RUN_TOKEN_SECRET: 'mock-run-token-binding-only-for-unit-tests',
  OWNER_STOP_TOKEN: 'mock-owner-stop-binding-only-for-unit-tests',
  VAST_API_KEY: 'mock-vast-binding',
};
const pair = await webcrypto.subtle.generateKey(
  {
    name: 'RSASSA-PKCS1-v1_5',
    modulusLength: 2048,
    publicExponent: new Uint8Array([1, 0, 1]),
    hash: 'SHA-256',
  },
  true,
  ['sign', 'verify'],
);
export const jwk = {
  ...(await webcrypto.subtle.exportKey('jwk', pair.publicKey)),
  kid: 'test-key',
  alg: 'RS256',
  use: 'sig',
};
export const baseClaims = {
  iss: 'https://token.actions.githubusercontent.com',
  aud: env.WORKER_URL,
  exp: clock / 1000 + 300,
  nbf: clock / 1000 - 1,
  repository: repo,
  repository_id: env.REPOSITORY_GITHUB_ID,
  repository_owner_id: owner,
  sub: 'repo:' + repo + ':environment:gpu-vast',
  job_workflow_ref: repo + '/.github/workflows/gpu-vast.yml@refs/heads/master',
  iat: clock / 1000,
  event_name: 'pull_request_target',
  actor_id: owner,
  sha,
  ref: 'refs/heads/master',
  run_id: '123',
  run_attempt: '1',
  workflow_ref: 'reindertpelsma/kayfabe/.github/workflows/gpu-vast.yml@refs/heads/master',
  workflow_sha: sha,
};
export async function jwt(patch = {}, header = {}, key = pair.privateKey) {
  const enc = (v) => Buffer.from(JSON.stringify(v)).toString('base64url');
  const input =
    enc({ alg: 'RS256', kid: 'test-key', ...header }) +
    '.' +
    enc({
      ...baseClaims,
      ...(!Object.hasOwn(patch, 'iat') && patch.nbf !== undefined
        ? { iat: Math.floor(patch.nbf + 1) }
        : {}),
      ...(patch.workflow_ref !== undefined && !Object.hasOwn(patch, 'job_workflow_ref')
        ? { job_workflow_ref: patch.workflow_ref }
        : {}),
      ...patch,
    });
  const sig = await webcrypto.subtle.sign(
    'RSASSA-PKCS1-v1_5',
    key,
    new TextEncoder().encode(input),
  );
  return input + '.' + Buffer.from(sig).toString('base64url');
}
export class MemoryStorage {
  data = new Map();
  async get(key) {
    return structuredClone(this.data.get(key));
  }
  async put(key, value) {
    this.data.set(key, structuredClone(value));
  }
  async delete(key) {
    return this.data.delete(key);
  }
  async list({ prefix = '', limit = 1000 } = {}) {
    return new Map(
      [...this.data]
        .filter(([k]) => k.startsWith(prefix))
        .sort(([a], [b]) => a.localeCompare(b))
        .slice(0, limit)
        .map(([k, v]) => [k, structuredClone(v)]),
    );
  }
}
export class MockCloud {
  credit = 20;
  price = 0.2;
  creates = 0;
  deletes = [];
  inspected = [];
  calls = [];
  ambiguous = false;
  githubError = 0;
  vastError = 0;
  logReady = true;
  logText = 'building\n';
  logStatus = 200;
  logHeaders = {};
  run = {
    id: 123,
    run_attempt: 1,
    event: 'pull_request_target',
    head_sha: sha,
    head_branch: 'owner-pr',
    pull_requests: [{ number: 7, head: { sha } }],
    actor: { id: 42 },
    triggering_actor: { id: 42 },
    status: 'in_progress',
    repository: { full_name: repo },
    head_repository: { full_name: repo },
  };
  prs = [
    {
      number: 7,
      state: 'open',
      draft: false,
      user: { id: 42 },
      head: { sha, ref: 'owner-pr', repo: { full_name: repo } },
      base: { ref: 'master', repo: { full_name: repo } },
      changed_files: 1,
    },
  ];
  reviews = [];
  reviewPages = null;
  files = [{ filename: 'crates/kf-core/src/lib.rs', status: 'modified' }];
  reporterText = reporterSource;
  reporterStatus = 200;
  reporterEncoding = 'base64';
  filePages = null;
  filesError = 0;
  offer = {
    id: 5,
    verified: true,
    vms_enabled: true,
    rentable: true,
    rented: false,
    num_gpus: 1,
    gpu_name: 'RTX_4070',
    gpu_arch: 'nvidia',
    compute_cap: 890,
    machine_id: 100001,
    driver_version: '580.95.05',
    dlperf: 30,
    cpu_cores_effective: 16,
    cpu_ghz: 3,
    cpu_ram: 65536,
    inet_down: 1000,
    inet_up: 100,
    disk_bw: 500,
    storage_cost: 0.12,
    inet_down_cost: 0,
    inet_up_cost: 0,
    reliability: 0.999,
    disk_space: 200,
  };
  // Concurrency harness: further offers, the contract ids handed out in order (default 900)
  // and per-run-id GitHub run rows (default `run`).
  extraOffers = [];
  contracts = null;
  created = [];
  runs = new Map();
  instances = new Map([
    ['777', { id: 777, label: 'owner-dev' }],
    ['888', { id: 888, label: 'kf-ci-unrecorded' }],
  ]);
  fetch = async (input, init = {}) => {
    const u = new URL(String(input));
    const method = init.method || 'GET';
    const body = init.body ? JSON.parse(init.body) : undefined;
    this.calls.push({ url: u.href, method, body, headers: new Headers(init.headers) });
    u.pathname = u.pathname.replace(/\/$/, ''); // Mock routing tolerates canonical provider slash.
    if (u.hostname === 'token.actions.githubusercontent.com') return Response.json({ keys: [jwk] });
    if (u.hostname === 'api.github.com') {
      if (this.githubError) return new Response('PRIVATE-UPSTREAM', { status: this.githubError });
      if (/\/contents\/scripts\/ci\/vast_report\.py$/.test(u.pathname)) {
        if (this.reporterStatus !== 200) return new Response('{}', { status: this.reporterStatus });
        const content = Buffer.from(this.reporterText).toString('base64');
        return Response.json({
          type: 'file',
          path: 'scripts/ci/vast_report.py',
          size: Buffer.byteLength(this.reporterText),
          encoding: this.reporterEncoding,
          content: content.replace(/(.{60})/g, '$1\n'),
        });
      }
      if (/\/actions\/runs\/\d+$/.test(u.pathname))
        return Response.json(this.runs.get(u.pathname.split('/').at(-1)) ?? this.run);
      if (/\/commits\/[a-f0-9]+$/.test(u.pathname))
        return Response.json({ sha: this.run.head_sha });
      if (/\/pulls\/\d+\/files$/.test(u.pathname)) {
        if (this.filesError) return new Response('PRIVATE-UPSTREAM', { status: this.filesError });
        return Response.json(
          this.filePages?.[Number(u.searchParams.get('page') || 1) - 1] ?? this.files,
        );
      }
      if (/\/commits\/[a-f0-9]+\/pulls$/.test(u.pathname)) return Response.json(this.prs);
      if (/\/pulls\/[0-9]+$/.test(u.pathname))
        return Response.json(
          this.prs.find((p) => String(p.number) === u.pathname.split('/').at(-1)),
        );
      if (/\/pulls\/\d+\/reviews$/.test(u.pathname))
        return Response.json(
          this.reviewPages?.[Number(u.searchParams.get('page') || 1) - 1] || this.reviews,
        );
      throw Error('Unexpected GitHub endpoint ' + u.pathname);
    }
    if (u.hostname === 's3.amazonaws.com') {
      if (!this.logReady) return new Response('', { status: 404 });
      return new Response(this.logText, { status: this.logStatus, headers: this.logHeaders });
    }
    if (u.hostname !== 'console.vast.ai') throw Error('Unapproved external host');
    if (this.vastError) return new Response('PRIVATE-UPSTREAM', { status: this.vastError });
    if (u.pathname === '/api/v0/users/current')
      return Response.json({ credit: this.credit, balance: 0 });
    if (u.pathname === '/api/v0/bundles') {
      if (body.type !== 'ondemand') return new Response('invalid type', { status: 400 });
      const row = (offer, price) => ({
        dph_base: typeof price === 'number' ? price - (offer.storage_cost * 120) / 720 : price,
        ...offer,
        dph_total: price,
      });
      return Response.json({
        offers: [
          row(this.offer, this.price),
          // Optional further market rows (concurrency and identity tests); each may carry
          // its own `price` (USD per hour, storage included).
          ...this.extraOffers.map(({ price, ...offer }) => row(offer, price ?? this.price)),
        ],
      });
    }
    if (/^\/api\/v0\/asks\/[1-9][0-9]*$/.test(u.pathname)) {
      this.creates++;
      const ask = Number(u.pathname.split('/').at(-1)),
        extra = this.extraOffers.find((x) => x.id === ask),
        offer = extra ?? this.offer,
        price = extra?.price ?? this.price,
        contract = this.contracts?.length ? this.contracts.shift() : 900;
      this.created.push({ ask, contract, label: body.label });
      this.instances.set(String(contract), {
        id: contract,
        label: body.label,
        onstart: body.onstart,
        image_uuid: body.image,
        actual_status: 'running',
        dph_base: price - (offer.storage_cost * 120) / 720,
        dph_total: price,
        storage_cost: offer.storage_cost,
        inet_down_cost: offer.inet_down_cost,
        inet_up_cost: offer.inet_up_cost,
      });
      if (this.ambiguous) throw Error('PRIVATE-UPSTREAM');
      return Response.json({
        success: true,
        new_contract: contract,
        instance_api_key: 'PRIVATE-UPSTREAM',
      });
    }
    if (u.pathname === '/api/v1/instances')
      return Response.json({
        success: true,
        instances: [...this.instances.values()],
        next_token: null,
      });
    const logs = u.pathname.match(/^\/api\/v0\/instances\/request_logs\/(\d+)$/);
    if (logs)
      return Response.json({
        success: true,
        result_url: this.logUrl || 'https://s3.amazonaws.com/vast.ai/instance_logs/mock.log',
      });
    const m = u.pathname.match(/^\/api\/v0\/instances\/(\d+)$/);
    if (!m) throw Error('Unexpected instance endpoint ' + u.pathname);
    if (method === 'DELETE') {
      this.deletes.push(m[1]);
      this.instances.delete(m[1]);
      return Response.json({ success: true });
    }
    this.inspected.push(m[1]);
    return this.instances.has(m[1])
      ? Response.json({ instances: this.instances.get(m[1]) })
      : new Response('', { status: 404 });
  };
}
// Run tokens minted independently of jose (node:crypto HMAC) so tests cross-check jose.
export function callbackToken(record, secret = env.RUN_TOKEN_SECRET, patch = {}) {
  const r = { ...record, ...patch },
    exp = r.reportExpiry ?? Math.floor((r.started + r.ttl) / 1000);
  const b64 = (v) => Buffer.from(JSON.stringify(v)).toString('base64url');
  const input =
    b64({ alg: 'HS256' }) +
    '.' +
    b64({ run: r.run, attempt: r.attempt, sha: r.sha, nonce: r.nonce, exp });
  return input + '.' + createHmac('sha256', secret).update(input).digest('base64url');
}
// The runner's /rent body (runner.py execute). Tests build theirs here so a protocol change of
// the body lands in one place.
export const rentBody = (tested_sha, extra = {}) => ({
  runner_nonce: 'f'.repeat(64),
  tested_sha,
  profile: 'check',
  ...extra,
});
export async function setup(overrides = {}) {
  const { createService } = await import('../src/index.js');
  const cloud = new MockCloud();
  const storage = new MemoryStorage();
  const time = { value: clock };
  const implicitPrefix = !Object.hasOwn(overrides, 'BROKER_PATH_PREFIX');
  const bindings = { ...env, BROKER_PATH_PREFIX: 'unit-fixture-' + 'p'.repeat(32), ...overrides };
  let service = createService({
    env: bindings,
    storage,
    fetch: cloud.fetch,
    now: () => time.value,
    random: () => nonce,
  });
  const transport = {
    fetch(request) {
      const url = new URL(request.url);
      if (implicitPrefix) {
        url.pathname = '/' + bindings.BROKER_PATH_PREFIX + url.pathname;
        request = new Request(url, request);
      }
      return service.fetch(request);
    },
    sweep: (...args) => service.sweep(...args),
  };
  const harness = {
    cloud,
    storage,
    time,
    bindings,
    get service() {
      return transport;
    },
    restart() {
      service = createService({
        env: bindings,
        storage,
        fetch: cloud.fetch,
        now: () => time.value,
        random: () => nonce,
      });
    },
    async report(payload, { run = '123-1', token, raw, headers = {} } = {}) {
      const record = await storage.get('record:' + run);
      if (
        payload?.kind === 'result' &&
        record?.runnerNonce &&
        payload.result?.suites === undefined &&
        payload.result?.hardware === undefined
      )
        payload = {
          ...payload,
          result: {
            ...payload.result,
            suites: Object.entries({
              provision: 1,
              kf_tests: 100,
              gates: 9,
              kf3_build: 1,
              raw_suite: 30,
              thin_suite: 30,
              birth_census: 1,
              cuda_host: 4,
              cuda_fat: 4,
            }).map(([name, pass]) => ({
              name,
              pass: payload.result?.verdict === 'PASS' ? pass : 0,
              fail: payload.result?.verdict === 'PASS' ? 0 : 1,
              skip: 0,
            })),
            hardware: { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' },
          },
        };
      if (payload?.kind === 'result' && record?.runnerNonce)
        payload = {
          ...payload,
          result: { runner_nonce: record.runnerNonce, sha: record.sha, ...payload.result },
        };
      const bearer = token ?? callbackToken(record, bindings.RUN_TOKEN_SECRET);
      return transport.fetch(
        new Request(bindings.WORKER_URL + '/report/' + run, {
          method: 'POST',
          headers: {
            Authorization: 'Bearer ' + bearer,
            'Content-Type': 'application/json',
            ...headers,
          },
          body: raw ?? JSON.stringify(payload),
        }),
      );
    },
    async call(
      path = '/rent',
      {
        claims = {},
        body,
        method = path.startsWith('/status') ? 'GET' : 'POST',
        token,
        ghToken = null,
        protocol = '6',
      } = {},
    ) {
      if (path === '/rent' && body === undefined)
        body = rentBody(
          cloud.run.event === 'pull_request_target'
            ? (cloud.run.pull_requests?.[0]?.head.sha ?? cloud.run.head_sha)
            : cloud.run.head_sha,
        );
      const authorization =
        token ??
        (await jwt({
          exp: time.value / 1000 + 300,
          nbf: time.value / 1000 - 1,
          iat: Math.floor(time.value / 1000),
          sha: cloud.run.head_sha,
          workflow_sha: cloud.run.head_sha,
          ...claims,
        }));
      return transport.fetch(
        new Request(bindings.WORKER_URL + path, {
          method,
          headers: {
            Authorization: 'Bearer ' + authorization,
            ...(protocol === null ? {} : { 'X-KF-Protocol': protocol }),
            ...(ghToken ? { 'X-GH-Token': ghToken } : {}),
          },
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        }),
      );
    },
  };
  return harness;
}
export const finalLine = (verdict = 'PASS', n = nonce) =>
  `KF_RESULT ${n} ${verdict} ${'c'.repeat(64)}\n`;

export const sampleResult = () => ({
  suites: Object.entries({
    provision: 1,
    kf_tests: 100,
    gates: 9,
    kf3_build: 1,
    raw_suite: 30,
    thin_suite: 30,
    birth_census: 1,
    cuda_host: 4,
    cuda_fat: 4,
  }).map(([name, pass]) => ({ name, pass, fail: 0, skip: 0 })),
  hardware: { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' },
});
