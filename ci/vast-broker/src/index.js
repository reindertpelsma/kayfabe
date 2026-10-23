// prettier-ignore
import {DEFAULTS} from './defaults.js';
// prettier-ignore
import {BrokerError,refuse,configuration,numericId,hexSha,localIdentity,readBounded} from './policy.js';
// prettier-ignore
import {RunTokens,constantTimeEqual,validSecret,sha256Hex} from './tokens.js';
// prettier-ignore
import {readReportBody,parseReport} from './reports.js';
// prettier-ignore
import {routePath,notFound,endpointSecrets,routeKind,secretFormatError} from './routes.js';
// prettier-ignore
import {OidcVerifier} from './oidc.js';
// prettier-ignore
import {Github} from './github.js';
// prettier-ignore
import {allowedWorkflowRef} from './workflow-authority.js';
// prettier-ignore
import {mintBlob,checkBlobBudget} from './blobs.js';
import { offerPrice, providerId } from './policy.js';
// prettier-ignore
import {Vast} from './vast.js';
// prettier-ignore
import {OfferBook} from './offers.js';
// prettier-ignore
import {Ledger} from './ledger.js';
// prettier-ignore
import {onstart} from './onstart.js';
// prettier-ignore
import {appendBoxText,appendWorkerText,sanitizeBoxText} from './sanitize.js';
// Signed facts only: repository, environment subject, master ref, run identity and a pinned
// master workflow. Who triggered the run and which commit it tests come from one run lookup.
function verifiedIdentity(c, cfg) {
  if (
    c.repository !== cfg.repo ||
    c.repository_id !== cfg.repositoryId ||
    c.repository_owner_id !== cfg.owner ||
    c.sub !== cfg.oidcSubject ||
    c.ref !== 'refs/heads/master' ||
    !numericId(c.actor_id) ||
    !numericId(c.run_id) ||
    !numericId(c.run_attempt) ||
    !hexSha(c.sha) ||
    !allowedWorkflowRef(c, cfg)
  )
    refuse('UNAUTHORIZED', 401);
  return c;
}
export function createService({
  env,
  storage,
  fetch: fetcher = fetch,
  now = Date.now,
  random,
} = {}) {
  const ledger = new Ledger(storage),
    verifier = new OidcVerifier(fetcher, now),
    tokens = new RunTokens(env.RUN_TOKEN_SECRET, now),
    githubCache = new Map(),
    offers = new OfferBook(now);
  let busy = false,
    localVerifier,
    admissionWindow = -1,
    admissionCount = 0;
  const runKey = (c) => c.run_id + '-' + c.run_attempt;
  const json = (data, status = 200) =>
    Response.json(data, {
      status,
      headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
    });
  const secrets = [
    env.BLOB_GITHUB_TOKEN,
    env.GITHUB_API_TOKEN,
    env.VAST_API_KEY,
    env.RUN_TOKEN_SECRET,
    env.OWNER_STOP_TOKEN,
    env.BOX_ACCESS_ID,
    env.BOX_ACCESS_SECRET,
    ...endpointSecrets(env),
  ].filter(Boolean);
  // Newly created real provider rows can omit/null actual_status. This grants
  // only the existing provisioning window, never callback/liveness authority.
  const pendingInstance = (i) =>
    i.actual_status == null || ['created', 'loading', 'starting'].includes(i.actual_status);
  const phase = (r) =>
    r.ended !== null
      ? 'NONE'
      : (r.phase ??
        (r.resultAt !== undefined
          ? 'GRACE'
          : r.firstReportAt !== undefined
            ? 'TESTING'
            : r.id
              ? 'BOOTING'
              : 'CREATING'));
  function clocks(r, cfg) {
    r.phase = phase(r);
    r.bootDeadline ??= r.started + (r.provision ?? cfg.boot);
    if (r.hardDeadline === undefined) r.hardDeadline = r.started + (r.ttl ?? cfg.hard);
    r.softDeadline ??= r.started + (r.ttl ?? cfg.hard) - cfg.softMargin;
    r.lastPostAt ??= r.firstReportAt ?? null;
    r.heartbeatTimeout ??= cfg.heartbeatTimeout;
    if (r.resultAt !== undefined && r.resultDeadline === undefined)
      r.resultDeadline = r.resultAt + (r.verdict === 'PASS' ? cfg.passGrace : cfg.failGrace);
  }
  function nextDeadline(r) {
    const values = [r.hardDeadline];
    if (r.resultAt !== undefined) values.push(r.resultDeadline);
    else if (r.firstReportAt === undefined) values.push(r.bootDeadline);
    else values.push(r.lastPostAt + r.heartbeatTimeout);
    return Math.min(...values.filter((x) => Number.isSafeInteger(x)));
  }
  const expose = (r, key = r.key) => ({
    run: key,
    id: r.id,
    machine_id: providerId(r.machineId),
    host_id: providerId(r.hostId),
    sha: r.sha,
    state: phase(r),
    verdict:
      r.ended !== null || r.resultAt !== undefined ? r.verdict || 'UNAVAILABLE' : 'UNAVAILABLE',
    cleanup_pending: r.ended === null,
    result_at: r.resultAt ?? null,
    ended_at: r.ended,
    started_at: r.started,
    deadline_at: Number.isFinite(nextDeadline(r)) ? nextDeadline(r) : r.started,
    gpu_model: r.hardware?.gpu_name ?? r.gpu ?? 'unknown',
    cost_estimate_usd:
      ((r.computeStoragePrice ?? r.price) * ((r.ended ?? now()) - r.started)) / 3600000 +
      (r.bandwidthCost ?? 0),
    log_cursor: r.logChunks ?? 0,
    log_tail_sanitised: r.log || '',
    bytes_dropped: r.fullBytesDropped ?? r.bytesDropped ?? 0,
    tail_bytes_dropped: r.bytesDropped || 0,
    runner_nonce: r.runnerNonce ?? '',
    log_sha256: r.logHash ?? '0'.repeat(64),
    profile: r.profile ?? 'check',
    die: r.die ?? 'unknown',
    country: r.country ?? 'UNKNOWN',
    compute_cap: r.hardware?.compute_cap ?? r.computeCap ?? 0,
    driver_version: r.hardware?.driver_version ?? r.driver ?? 'unknown',
    selection_reason: r.selectionReason ?? 'NONE',
    preferences_ignored: r.preferencesIgnored ?? 0,
    suites: r.result?.suites ?? [],
    boot_deadline_at: r.bootDeadline ?? null,
    hard_deadline_at: r.hardDeadline ?? null,
    soft_deadline_at: r.softDeadline ?? null,
    last_post_at: r.lastPostAt ?? null,
    result_deadline_at: r.resultDeadline ?? null,
    protocol_version: DEFAULTS.statusProtocol,
    worker_version: 'v5',
    bundle_sha256: env.DEPLOYED_BUNDLE_SHA256 ?? '0'.repeat(64),
  });
  async function workerLine(r, message, cfg) {
    const text = '[Worker] ' + message + '\n',
      bytes = new TextEncoder().encode(text).length;
    if ((r.fullBytes ?? 0) + bytes > cfg.reportBytes) {
      r.fullBytesDropped = (r.fullBytesDropped ?? 0) + bytes;
      await ledger.save(r);
      return;
    }
    await ledger.appendLog(r, text);
    r.fullBytes = (r.fullBytes ?? 0) + bytes;
    appendWorkerText(r, text);
    await ledger.save(r);
  }
  async function rejectExpiredCallback(r, cfg, vast) {
    if (now() >= nextDeadline(r)) {
      await workerLine(r, 'destroy reason=DEADLINE callback', cfg);
      try {
        await close(r, vast, 'DEADLINE');
      } catch {}
      refuse('UNAUTHORIZED', 401);
    }
  }
  async function clearFailedSha(record) {
    const cached = await ledger.get('sha:' + record.sha);
    if (cached?.key === record.key) await ledger.storage.delete('sha:' + record.sha);
  }
  async function recover(record, vast) {
    if (!record.id && record.ended === null) {
      record.id = await vast.reconcile(record);
      if (record.id === null) {
        const previous = record.noContractAt;
        record.noContractAt ??= now();
        const deadline =
          record.bootDeadline ??
          record.started + (record.provision ?? DEFAULTS.bootMinutes * 60000);
        if (
          previous !== undefined &&
          now() >= deadline &&
          now() - previous >= DEFAULTS.cronPeriod
        ) {
          record.closed = 'GONE';
          record.verdict ||= 'UNAVAILABLE';
          record.computeStoragePrice = 0;
          record.price = 0;
          record.bandwidthCost = 0;
          await ledger.settle(record, now());
          await clearFailedSha(record);
          return record;
        }
      } else {
        delete record.noContractAt;
        record.idSource = 'reconcile';
      }
      await ledger.save(record);
    }
    return record;
  }
  async function cleanup(record, vast) {
    await recover(record, vast);
    if (record.ended !== null || !record.id) return;
    await vast.destroy(record);
    await ledger.settle(record, now());
    console.log(JSON.stringify({ event: 'destroyed' }));
  }
  async function close(record, vast, reason, verdict = 'FAIL') {
    record.closed = reason;
    record.verdict ||= verdict;
    record.liveCheckedAt = null;
    await ledger.save(record);
    if (record.ended === null) await cleanup(record, vast);
  }
  async function checkContract(record, instance, vast, cfg) {
    const price = instance && offerPrice(instance, cfg);
    const cap = record.profile === 'sweep' ? cfg.sweepPrice : cfg.price;
    if (
      !price ||
      instance.inet_up_cost > cfg.maxInet ||
      instance.inet_down_cost > cfg.maxInet ||
      price.effective > cap
    ) {
      await close(record, vast, 'CONTRACT_PRICE', 'UNAVAILABLE');
      refuse('UNAVAILABLE', 503);
    }
    // Retain the highest observed components: a later decrease cannot erase past liability.
    const computeStorage = Math.max(record.computeStoragePrice ?? 0, price.computeStorage);
    const bandwidth = Math.max(record.bandwidthCost ?? 0, price.bandwidth);
    if (computeStorage !== record.computeStoragePrice || bandwidth !== record.bandwidthCost) {
      const ttl = record.billingTtl ?? record.ttl + cfg.cronPeriod;
      const total = (computeStorage * ttl) / 3600000 + bandwidth;
      try {
        const credit = await vast.credit();
        if (typeof credit !== 'number' || !Number.isFinite(credit) || credit < cfg.reserve + total)
          refuse('UNAVAILABLE_BUDGET', 409);
        await ledger.budgets(cfg, now(), total, record.key);
      } catch {
        await close(record, vast, 'CONTRACT_BUDGET', 'UNAVAILABLE');
        refuse('UNAVAILABLE_BUDGET', 409);
      }
      record.computeStoragePrice = computeStorage;
      record.bandwidthCost = bandwidth;
      record.price = computeStorage + bandwidth / (cfg.hard / 3600000);
      await ledger.save(record);
      await ledger.put('cost:' + record.key, {
        profile: record.profile ?? 'check',
        started: record.started,
        ended: null,
        ttl,
        price: computeStorage,
        bandwidthCost: bandwidth,
      });
    }
  }
  async function update(record, vast, github, cfg) {
    cfg.profile = record.profile ?? 'check';
    if (record.ended !== null) return;
    clocks(record, cfg);
    await ledger.save(record);
    await recover(record, vast);
    if (record.ended !== null || !record.id) return;
    if (record.closed) {
      await cleanup(record, vast);
      return;
    }
    let reason;
    if (record.hardDeadline !== null && now() >= record.hardDeadline) reason = 'HARD_TIMEOUT';
    else if (record.resultAt !== undefined && now() >= record.resultDeadline) reason = 'RESULT';
    else if (record.firstReportAt === undefined && now() >= record.bootDeadline)
      reason = 'BOOT_TIMEOUT';
    else if (record.lastPostAt !== null && now() - record.lastPostAt >= record.heartbeatTimeout)
      reason = 'HEARTBEAT_TIMEOUT';
    if (reason) {
      if (
        reason === 'BOOT_TIMEOUT' &&
        record.providerState === 'created' &&
        record.bootFailure?.confirmed &&
        now() - record.bootFailure.last <= cfg.cronPeriod * 2 &&
        Number.isSafeInteger(record.machineId)
      ) {
        await ledger.put('bad-machine:' + record.machineId, {
          reason: 'KVM_DOMAIN_NOT_FOUND',
          at: now(),
        });
        await workerLine(record, 'host excluded after frozen KVM boot signature', cfg);
      }
      await workerLine(record, 'destroy reason=' + reason, cfg);
      await close(record, vast, reason, reason === 'BOOT_TIMEOUT' ? 'UNAVAILABLE' : 'FAIL');
      return;
    }
    let instance;
    try {
      instance = await vast.inspect(record);
    } catch {
      await workerLine(record, 'provider_error', cfg);
      record.closed = 'UNAVAILABLE';
      record.verdict ||= 'UNAVAILABLE';
      await ledger.save(record);
      throw Error('UNAVAILABLE');
    }
    if (instance === null) {
      record.closed = 'GONE';
      record.verdict ||= 'UNAVAILABLE';
      await workerLine(record, 'provider_absent', cfg);
      await ledger.settle(record, now());
      return;
    }
    await checkContract(record, instance, vast, cfg);
    const providerState = [
      'running',
      'created',
      'loading',
      'starting',
      'stopped',
      'exited',
    ].includes(instance.actual_status)
      ? instance.actual_status
      : 'unknown';
    if (record.providerState !== providerState) {
      record.providerState = providerState;
      await workerLine(record, 'provider=' + providerState + ' phase=' + record.phase, cfg);
    }
    if (instance.actual_status === 'created' && record.firstReportAt === undefined) {
      // Provider diagnostics never decide test PASS and never postpone cleanup.
      try {
        const log = await vast.bootLog(record),
          signature = "Domain not found: no domain with matching name 'C." + record.id + "'";
        if (
          typeof log === 'string' &&
          log.includes('libvirt: QEMU Driver error') &&
          log.includes(signature)
        ) {
          const hash = await sha256Hex(log),
            prior = record.bootFailure;
          record.bootFailure =
            prior?.hash === hash
              ? {
                  ...prior,
                  last: now(),
                  confirmed: now() - prior.first >= DEFAULTS.bootFailureFreezeMs,
                }
              : { hash, first: now(), last: now(), confirmed: false };
        } else record.bootFailure = null;
      } catch {
        record.bootFailure = null;
      }
      await ledger.save(record);
    } else {
      record.bootFailure = null;
      await ledger.save(record);
    }
    if (instance.actual_status !== 'running') {
      if (record.firstReportAt === undefined && pendingInstance(instance)) return;
      await close(record, vast, 'ENDED');
      return;
    }
    record.liveCheckedAt = now();
    await ledger.save(record);
    // Post-result inspection outlives the workflow job; before it, the run must be live.
    if (record.resultAt !== undefined) return;
    const run = await github.run(record.run, true, true);
    if (run === null || run.status === 'completed') await close(record, vast, 'RUN_GONE');
  }
  async function oidc(bearer, cfg) {
    const selected = cfg.testIdentity
      ? (localVerifier ??= new OidcVerifier(fetcher, now, cfg.testIdentity))
      : verifier;
    return verifiedIdentity(await selected.verify(bearer, cfg.audience), cfg);
  }
  async function rate(r, cfg) {
    const window = Math.floor(now() / 10000);
    if (r.rateWindow !== window) {
      r.rateWindow = window;
      r.rateCount = 0;
    }
    if (r.rateCount >= cfg.reportRate) refuse('RATE_LIMITED', 429);
    r.rateCount++;
    await ledger.save(r);
  }
  async function kill(request, key, bearer, cfg, vast) {
    const r = await ledger.get('record:' + key);
    if (!r || r.key !== key || !(await tokens.verify(bearer, r)) || now() >= r.reportExpiry * 1000)
      refuse('UNAUTHORIZED', 401);
    // An ended kill receipt is idempotent but grants no live authority or provider calls.
    if (r.ended !== null) {
      if (r.boxKilled) return json({ state: 'NONE' });
      refuse('UNAUTHORIZED', 401);
    }
    if (r.closed && !r.boxKilled) refuse('UNAUTHORIZED', 401);
    await rate(r, cfg);
    await readBounded(request, 256);
    r.boxKilled = true;
    await close(r, vast, 'STOPPED');
    return json({ state: 'NONE' });
  }
  async function report(request, key, bearer, cfg, vast) {
    const r = await ledger.get('record:' + key);
    if (
      !(await tokens.verify(bearer, r)) ||
      !r ||
      r.key !== key ||
      r.ended !== null ||
      r.closed ||
      r.resultAt !== undefined
    )
      refuse('UNAUTHORIZED', 401);
    clocks(r, cfg);
    await rejectExpiredCallback(r, cfg, vast);
    if (now() >= r.reportExpiry * 1000 || (r.hardDeadline !== null && now() >= r.hardDeadline)) {
      try {
        await close(r, vast, 'TTL');
      } catch {}
      refuse('UNAUTHORIZED', 401);
    }
    if (r.reportCount >= cfg.reportCount || r.reportBytes >= cfg.reportBytes) {
      try {
        await close(r, vast, 'LIMIT');
      } catch {}
      refuse('UNAUTHORIZED', 401);
    }
    await rate(r, cfg);
    r.reportCount++;
    try {
      if (
        r.liveCheckedAt === null ||
        r.liveCheckedAt === undefined ||
        now() - r.liveCheckedAt >= 3000
      ) {
        try {
          await recover(r, vast);
          const instance = await vast.inspect(r);
          if (instance && r.firstReportAt === undefined && pendingInstance(instance))
            refuse('NOT_READY', 409);
          if (instance === null || instance.actual_status !== 'running') throw Error('NOT_LIVE');
          r.liveCheckedAt = now();
        } catch (error) {
          if (error instanceof BrokerError && error.state === 'NOT_READY') throw error;
          try {
            await close(r, vast, 'UNAVAILABLE', 'UNAVAILABLE');
          } catch {}
          refuse('UNAUTHORIZED', 401);
        }
      }
      const raw = await readReportBody(request, r, cfg),
        payload = parseReport(raw);
      const digest = await sha256Hex(JSON.stringify(payload));
      if (payload.seq <= r.lastSeq) {
        if (payload.seq === r.lastSeq && digest === r.lastDigest)
          return json({ state: 'ACCEPTED' });
        refuse('REPORT_SEQUENCE', 409);
      }
      if (payload.kind === 'result' && payload.result.nonce !== r.nonce) refuse('BAD_REPORT', 400);
      if (
        payload.kind === 'result' &&
        (payload.result.runner_nonce !== r.runnerNonce || payload.result.sha !== r.sha)
      )
        refuse('BAD_REPORT', 400);
      if (payload.text !== undefined) {
        const bytes = new TextEncoder().encode(payload.text),
          redactions = [...secrets, bearer, r.id, r.label, r.nonce];
        const safe = sanitizeBoxText(
          bytes,
          Math.max(0, cfg.reportBytes - (r.fullBytes ?? 0)),
          redactions,
          { tail: false },
        );
        await ledger.appendLog(r, safe.text);
        r.fullBytes = (r.fullBytes ?? 0) + new TextEncoder().encode(safe.text).length;
        r.fullBytesDropped = (r.fullBytesDropped ?? 0) + safe.bytesDropped;
        appendBoxText(r, bytes, redactions);
      }
      r.lastSeq = payload.seq;
      r.lastDigest = digest;
      r.firstReportAt ??= now();
      r.lastPostAt = now();
      if (r.phase !== 'GRACE') r.phase = 'TESTING';
      if (payload.kind === 'result') {
        r.result = {
          ...payload.result,
          die: r.die,
          profile: r.profile,
          selection_reason: r.selectionReason,
          preferences_ignored: r.preferencesIgnored,
        };
        r.hardware = payload.result.hardware;
        if (r.profile === 'sweep' && payload.result.verdict === 'PASS')
          await ledger.put('passed:' + r.die, now());
        r.resultAt = now();
        r.verdict = payload.result.verdict;
        r.grace = r.verdict === 'PASS' ? cfg.passGrace : cfg.failGrace;
        r.resultDeadline = r.resultAt + r.grace;
        r.phase = 'GRACE';
        r.liveCheckedAt = null;
      } else if (r.reportCount >= cfg.reportCount || r.reportBytes >= cfg.reportBytes)
        await close(r, vast, 'LIMIT');
      return json({
        state: 'ACCEPTED',
        ...(payload.kind === 'result'
          ? { grace_remaining_seconds: Math.max(0, Math.floor((nextDeadline(r) - now()) / 1000)) }
          : {}),
      });
    } catch (error) {
      if (r.reportBytes >= cfg.reportBytes && !r.closed) {
        try {
          await close(r, vast, 'LIMIT');
        } catch {}
      }
      throw error;
    } finally {
      await ledger.save(r);
    }
  }
  async function perform(request) {
    const url = new URL(request.url),
      path = await routePath(request, env);
    if (path === null) return notFound();
    if (path === '/rent' && env.RENTING_ENABLED !== 'true')
      return json(
        {
          state: 'RENTING_DISABLED',
          verdict: 'UNAVAILABLE',
          log_tail_sanitised: '',
          bytes_dropped: 0,
        },
        503,
      );
    const cfg = configuration(env);
    cfg.testIdentity = localIdentity(env, request.url);
    if (cfg.testIdentity) cfg.githubOrigin = cfg.testIdentity.githubOrigin;
    if (url.search && !/^\/logs\//.test(path) && path !== '/availability')
      refuse('BAD_REQUEST', 400);
    const blob =
      /^\/blob-url\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})\/([A-Za-z0-9._-]{1,64})\/([A-Za-z0-9._-]{1,100})$/.exec(
        path,
      );
    const match = /^\/(report|kill|status|stop|logs)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.exec(
      path,
    );
    if (
      !(blob && request.method === 'GET') &&
      !(path === '/availability' && request.method === 'GET') &&
      !(path === '/rent' && request.method === 'POST') &&
      !(path === '/stop-all' && request.method === 'POST') &&
      !(match && request.method === (['status', 'logs'].includes(match[1]) ? 'GET' : 'POST'))
    )
      return notFound();
    const authorization = request.headers.get('Authorization');
    if (!authorization?.startsWith('Bearer ')) refuse('UNAUTHORIZED', 401);
    const bearer = authorization.slice(7),
      vast = new Vast(fetcher, cfg, env.VAST_API_KEY);
    if (blob) {
      const r = await ledger.get('record:' + blob[1]);
      if (
        !r ||
        r.key !== blob[1] ||
        !(await tokens.verify(bearer, r)) ||
        r.ended !== null ||
        r.closed ||
        now() >= r.reportExpiry * 1000
      )
        refuse('UNAUTHORIZED', 401);
      clocks(r, cfg);
      await rejectExpiredCallback(r, cfg, vast);
      await readBounded(request, 0);
      await checkBlobBudget(env, ledger, r, blob[2], blob[3], now());
      await rate(r, cfg);
      const instance = await vast.inspect(r);
      if (!instance || instance.actual_status !== 'running') refuse('UNAUTHORIZED', 401);
      return json(await mintBlob(fetcher, env, ledger, r, blob[2], blob[3], now()));
    }
    if (match?.[1] === 'report') return report(request, match[2], bearer, cfg, vast);
    if (match?.[1] === 'kill') return kill(request, match[2], bearer, cfg, vast);
    let rentRequest;
    if (path !== '/rent' && request.body !== null) {
      try {
        await readBounded(request, 0);
      } catch {
        refuse('BAD_REQUEST', 400);
      }
    }
    const owner =
      validSecret(env.OWNER_STOP_TOKEN) && (await constantTimeEqual(env.OWNER_STOP_TOKEN, bearer));
    if (path === '/stop-all') {
      if (!owner) refuse('UNAUTHORIZED', 401);
      const records = await ledger.openRecords();
      for (const r of records) {
        r.closed = 'STOPPED';
        r.verdict ||= 'FAIL';
        r.liveCheckedAt = null;
        await ledger.save(r);
      }
      let failed = false;
      for (const r of records) {
        try {
          await cleanup(r, vast);
        } catch {
          failed = true;
        }
      }
      if (failed) refuse();
      return json({ state: 'NONE' });
    }
    if (match?.[1] === 'stop' && owner) {
      const r = (await ledger.granted(match[2])) || (await ledger.get('record:' + match[2]));
      if (!r) refuse('NOT_FOUND', 404);
      await close(r, vast, 'STOPPED');
      return json(expose(r, match[2]));
    }
    const c = await oidc(bearer, cfg),
      key = runKey(c);
    // Runner and Worker must speak the same status protocol; each side refuses the other.
    if (request.headers.get('X-KF-Protocol') !== String(DEFAULTS.statusProtocol))
      refuse('PROTOCOL_MISMATCH', 409);
    if (path === '/availability' || path === '/rent') {
      const window = Math.floor(now() / 60000);
      if (admissionWindow !== window) {
        admissionWindow = window;
        admissionCount = 0;
      }
      if (++admissionCount > DEFAULTS.admissionPerMinute) refuse('RATE_LIMIT', 429);
    }

    if (path === '/rent') {
      try {
        const text = new TextDecoder('utf-8', { fatal: true }).decode(
          await readBounded(request, 2048),
        );
        rentRequest = JSON.parse(text);
        // runner_nonce, tested_sha and profile are required; preferences is optional.
        const allowed = ['runner_nonce', 'tested_sha', 'profile', 'preferences'],
          keys = Object.keys(rentRequest),
          matches = [...text.matchAll(/"(runner_nonce|tested_sha|profile|preferences)"\s*:/g)];
        if (
          keys.length < 3 ||
          keys.length > 4 ||
          !keys.every((k) => allowed.includes(k)) ||
          !allowed.slice(0, 3).every((k) => keys.includes(k)) ||
          matches.length !== keys.length ||
          new Set(matches.map((x) => x[1])).size !== keys.length ||
          !/^([0-9a-f]{64})$/.test(rentRequest.runner_nonce) ||
          !/^([0-9a-f]{40})$/.test(rentRequest.tested_sha)
        )
          throw Error();
      } catch {
        refuse('BAD_REQUEST', 400);
      }
    }
    const token = env.GITHUB_API_TOKEN;
    if (token && (token.length > 1024 || !/^[\x21-\x7e]+$/.test(token))) refuse();
    const github = new Github(fetcher, cfg, token, { cache: githubCache, now });
    if (match) {
      if (match[2] !== key) refuse('UNAUTHORIZED', 401);
      const r = await ledger.granted(key);
      if (!r) refuse('NOT_FOUND', 404);
      if (match[1] === 'logs') {
        if (!/^\?cursor=(0|[1-9][0-9]{0,6})$/.test(url.search)) refuse('BAD_REQUEST', 400);
        const cursor = Number(url.searchParams.get('cursor'));
        if (cursor > (r.logChunks ?? 0)) refuse('BAD_REQUEST', 400);
        return json(await ledger.logPage(r, cursor));
      }
      if (match[1] === 'stop') await close(r, vast, 'STOPPED');
      else await update(r, vast, github, cfg);
      return json(expose(r, key));
    }
    const profile =
      path === '/availability' ? url.searchParams.get('profile') : rentRequest.profile;
    if (!['check', 'sweep'].includes(profile)) refuse('BAD_REQUEST', 400);
    if (path === '/availability' && url.search !== '?profile=' + profile)
      refuse('BAD_REQUEST', 400);
    if (!allowedWorkflowRef(c, cfg, profile)) refuse('UNAUTHORIZED', 401);
    cfg.profile = profile;
    if (path === '/availability') {
      await github.authorize(c);
      return json((await offers.evaluate(vast, cfg, ledger, profile)).view);
    }
    if (
      rentRequest.preferences !== undefined &&
      (!Array.isArray(rentRequest.preferences) ||
        rentRequest.preferences.length > 16 ||
        !rentRequest.preferences.every(
          (x) =>
            typeof x === 'string' &&
            (/^[A-Z][A-Z0-9_]{1,23}$/.test(x) || (profile === 'sweep' && x === 'rotate')),
        ) ||
        new Set(rentRequest.preferences).size !== rentRequest.preferences.length)
    )
      refuse('BAD_REQUEST', 400);
    const target = await github.authorize(c, rentRequest.tested_sha);
    const prior = await ledger.granted(key);
    if (prior) {
      if (prior.runnerNonce !== rentRequest.runner_nonce) refuse('BINDING_MISMATCH', 409);
      await recover(prior, vast);
      if (prior.id && prior.ended === null)
        await checkContract(prior, await vast.inspect(prior), vast, cfg);
      return json(expose(prior, key));
    }
    const active = await ledger.active();
    if (active && active.ended === null) {
      if (active.branch === target.branch && active.sha !== target.sha) {
        await close(active, vast, 'SUPERSEDED');
        refuse('BUSY', 409);
      }
      refuse('UNAVAILABLE_BUDGET', 409);
    }
    const cached = await ledger.get('sha:' + target.sha),
      force = Number(c.run_attempt) > 1; // An owner re-run; the SHA gap below still applies.
    if (cached && !force) refuse('UNAVAILABLE_BUDGET', 409); // A fresh runner nonce may not inherit another run's result.
    if (cached && now() - cached.started < cfg.gap) refuse('UNAVAILABLE_BUDGET', 409);
    if (!validSecret(env.RUN_TOKEN_SECRET) || !validSecret(env.OWNER_STOP_TOKEN)) refuse();
    await github.boxProtocol(target.sha);
    const chosen = await offers.select(
        await offers.evaluate(vast, cfg, ledger, profile),
        cfg,
        ledger,
        profile,
        rentRequest.preferences,
      ),
      offer = chosen.offer,
      cost = offer.pricing.total,
      credit = await vast.credit();
    try {
      if (typeof credit !== 'number' || !Number.isFinite(credit) || credit < cfg.reserve + cost)
        refuse('UNAVAILABLE_BUDGET', 409);
      await ledger.budgets(cfg, now(), cost);
    } catch (e) {
      if (e instanceof BrokerError && e.state === 'UNAVAILABLE_BUDGET')
        e.selection = { die: offer.die, ignored: chosen.ignored, reason: chosen.reason };
      throw e;
    }
    const n = random
      ? random()
      : Array.from(crypto.getRandomValues(new Uint8Array(16)), (x) =>
          x.toString(16).padStart(2, '0'),
        ).join('');
    const started = now(),
      r = {
        ...target,
        profile,
        die: offer.die,
        country: offer.country,
        computeCap: offer.compute_cap,
        machineId: offer.machine_id,
        hostId: providerId(offer.host_id),
        driver: offer.driver_version,
        preferencesIgnored: chosen.ignored,
        selectionReason: chosen.reason,
        bandwidthCost: offer.pricing.bandwidth,
        computeStoragePrice: offer.pricing.computeStorage,
        key,
        label: 'kf-ci-' + key,
        nonce: n,
        runnerNonce: rentRequest.runner_nonce,
        logHash: '0'.repeat(64),
        phase: 'CREATING',
        bootDeadline: started + cfg.boot,
        hardDeadline: started + cfg.hard,
        softDeadline: started + cfg.hard - cfg.softMargin,
        lastPostAt: null,
        heartbeatTimeout: cfg.heartbeatTimeout,
        started,
        ended: null,
        ttl: cfg.ttl,
        billingTtl: cfg.ttl + cfg.cronPeriod,
        provision: cfg.provision,
        grace: cfg.grace,
        gpu: offer.gpu_name,
        // Box capability lifetime: the hard limit (no route serves a box past it).
        reportExpiry: Math.floor((started + cfg.hard) / 1000),
        price: offer.pricing.effective,
        id: null,
        closed: null,
        verdict: null,
        log: '',
        bytesDropped: 0,
        reportCount: 0,
        reportBytes: 0,
        lastSeq: 0,
        lastDigest: null,
        rateWindow: 0,
        rateCount: 0,
        liveCheckedAt: null,
      };
    const runToken = await tokens.issue(r),
      script = onstart(target.sha, cfg.repo);
    r.scriptHash = await sha256Hex(script);
    await ledger.reserve(r);
    await workerLine(
      r,
      'selection profile=' +
        profile +
        ' die=' +
        r.die +
        ' reason=' +
        r.selectionReason +
        ' ignored=' +
        r.preferencesIgnored,
      cfg,
    );
    if (profile === 'sweep') await ledger.put('rotation:lastDie', r.die);
    try {
      const environment = {
        KF_PROTOCOL: DEFAULTS.boxProtocol,
        KF_BROKER_URL: cfg.audience + '/' + env.BROKER_PATH_PREFIX,
        KF_RUN_ID: key,
        KF_RUN_TOKEN: runToken,
        KF_NONCE: n,
        KF_SHA: r.sha,
        KF_REPO: cfg.repo,
        KF_RUNNER_NONCE: r.runnerNonce,
        KF_BOOT_DEADLINE: Math.floor(r.bootDeadline / 1000),
        KF_HARD_DEADLINE: Math.floor(r.hardDeadline / 1000),
        KF_SOFT_DEADLINE: Math.floor(r.softDeadline / 1000),
        KF_HEARTBEAT_SECONDS: cfg.heartbeat / 1000,
        KF_GRACE_PASS: cfg.passGrace / 1000,
        KF_GRACE_FAIL: cfg.failGrace / 1000,
        // Pass-through for the hostname-wide Access policy; the Worker never verifies Access.
        ...(env.BOX_ACCESS_ID && env.BOX_ACCESS_SECRET
          ? { KF_ACCESS_ID: env.BOX_ACCESS_ID, KF_ACCESS_SECRET: env.BOX_ACCESS_SECRET }
          : {}),
      };
      r.id = await vast.create(offer, r, script, environment);
      r.idSource = 'create';
      await ledger.save(r);
      await checkContract(r, await vast.inspect(r), vast, cfg);
    } catch (e) {
      if (e instanceof BrokerError && e.state === 'CREATE_REFUSED') {
        r.closed = 'GONE';
        r.verdict = 'UNAVAILABLE';
        r.computeStoragePrice = 0;
        r.price = 0;
        r.bandwidthCost = 0;
        await ledger.settle(r, now());
        await clearFailedSha(r);
      }
      refuse();
    }
    r.phase = 'BOOTING';
    await ledger.save(r);
    console.log(JSON.stringify({ event: 'rented' }));
    return json(expose(r));
  }
  const releaseUpload = async (request) => {
    if (request.body && !request.body.locked) await request.body.cancel().catch(() => {});
  };
  // Authentication may wait on JWKS, but never owns the ledger queue.
  let executing = false;
  const queue = [];
  async function serial(priority, action, key = '@cron') {
    if (
      key !== '@cron' &&
      (queue.length >= 32 ||
        queue.filter((x) => x.priority === priority).length >= 16 ||
        queue.filter((x) => x.key === key).length >= 2)
    )
      refuse('BUSY', 409);
    return new Promise((resolve, reject) => {
      queue.push({ priority, key, action, resolve, reject });
      queue.sort((a, b) => a.priority - b.priority);
      drain();
    });
  }
  async function drain() {
    if (executing) return;
    executing = true;
    try {
      while (queue.length) {
        const task = queue.shift();
        try {
          task.resolve(await task.action());
        } catch (e) {
          task.reject(e);
        }
      }
    } finally {
      executing = false;
    }
  }
  async function preauthenticate(request) {
    if (new URL(request.url).pathname.endsWith('/availability') && secretFormatError(env)) {
      const cfg = configuration(env);
      const c = await oidc(request.headers.get('Authorization')?.replace(/^Bearer /, ''), cfg);
      if (c.actor_id === cfg.owner) refuse(secretFormatError(env), 503);
    }
    const path = await routePath(request, env);
    if (path === null) return;
    const cfg = configuration(env);
    cfg.testIdentity = localIdentity(env, request.url);
    if (path === '/rent' && env.RENTING_ENABLED !== 'true') return;
    const bearer = request.headers.get('Authorization')?.replace(/^Bearer /, '');
    if (!bearer) refuse('UNAUTHORIZED', 401);
    if (path === '/stop-all' || /^\/stop\//.test(path)) {
      if (
        validSecret(env.OWNER_STOP_TOKEN) &&
        (await constantTimeEqual(env.OWNER_STOP_TOKEN, bearer))
      )
        return;
    }
    const box =
      /^\/(report|kill)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.exec(path) ??
      /^\/(blob-url)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})\/[A-Za-z0-9._-]{1,64}\/[A-Za-z0-9._-]{1,100}$/.exec(
        path,
      );
    if (box) {
      const record = await ledger.get('record:' + box[2]);
      if (!record || !(await tokens.verify(bearer, record))) refuse('UNAUTHORIZED', 401);
      if (record.ended !== null || record.closed) {
        if (box[1] === 'kill' && record.boxKilled && record.ended !== null)
          return json({ state: 'NONE' });
        refuse('UNAUTHORIZED', 401);
      }
      if (box[1] === 'report' && record.resultAt !== undefined) refuse('UNAUTHORIZED', 401);
      return;
    }
    if (path === '/rent' && env.RENTING_ENABLED !== 'true') return;
    if (
      !/^\/(?:rent|availability|stop-all|(?:status|logs|stop)\/[1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.test(
        path,
      )
    )
      return;
    await oidc(bearer, cfg);
  }
  return {
    async fetch(request) {
      try {
        const receipt = await preauthenticate(request);
        if (receipt instanceof Response) return receipt;
        const path = await routePath(request, env);
        const renting = path === '/rent';
        if (renting && busy) refuse('BUSY', 409);
        if (renting) busy = true;
        try {
          const priority =
            path === '/stop-all' || /^\/(?:stop|kill)\//.test(path)
              ? 0
              : /^\/report\//.test(path)
                ? 1
                : 2;
          const key = /\/([1-9][0-9]*-[1-9][0-9]*)/.exec(path)?.[1] ?? path;
          return await serial(priority, () => perform(request), key);
        } finally {
          if (renting) busy = false;
        }
      } catch (e) {
        return json(
          {
            state: e instanceof BrokerError ? e.state : 'UNAVAILABLE',
            verdict: 'UNAVAILABLE',
            log_tail_sanitised: '',
            bytes_dropped: 0,
            ...(e instanceof BrokerError && e.reason ? { reason: e.reason } : {}),
            ...(e instanceof BrokerError && e.selection ? { selection: e.selection } : {}),
          },
          e instanceof BrokerError ? e.status : 503,
        );
      } finally {
        await releaseUpload(request);
      }
    },
    async sweep() {
      return serial(0, async () => {
        try {
          const cfg = configuration(env),
            records = await ledger.openRecords();
          for (const r of records) {
            try {
              await update(
                r,
                new Vast(fetcher, cfg, env.VAST_API_KEY),
                new Github(fetcher, cfg, env.GITHUB_API_TOKEN, { cache: githubCache, now }),
                cfg,
              );
            } catch {
              console.log(JSON.stringify({ event: 'cleanup_unavailable' }));
            }
          }
          await ledger.expire(now(), cfg);
        } catch (e) {
          // A configuration refusal names only the variable; operators must not read silence.
          console.log(
            JSON.stringify(
              e instanceof BrokerError && /^CONFIG_[A-Z_]+:[A-Z_]+$/.test(e.state)
                ? { event: 'cleanup_config_error', state: e.state }
                : { event: 'cleanup_unavailable' },
            ),
          );
        }
      });
    },
  };
}
export class BrokerLedger {
  constructor(ctx, env) {
    this.service = createService({ env, storage: ctx.storage });
  }
  async fetch(request) {
    if (
      new URL(request.url).origin === 'https://internal.invalid' &&
      new URL(request.url).pathname === '/cron'
    ) {
      await this.service.sweep();
      return new Response('ok');
    }
    return this.service.fetch(request);
  }
}
// Buffer only the admitted prefix before entering the serialized ledger. An
// unfinished network upload must never hold its busy flag or block cron.
async function detachedUpload(request, path) {
  if (!request.body) return request;
  const limit =
    (/^\/report\//.test(path)
      ? 65536
      : /^\/kill\//.test(path)
        ? 256
        : path === '/rent'
          ? 2048
          : 0) + 1;
  const reader = request.body.getReader(),
    chunks = [];
  let length = 0;
  try {
    while (length < limit) {
      const { value, done } = await reader.read();
      if (done) break;
      const kept = value.slice(0, limit - length);
      chunks.push(kept);
      length += kept.byteLength;
    }
  } finally {
    await reader.cancel().catch(() => {});
  }
  const bytes = new Uint8Array(length);
  let at = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, at);
    at += chunk.byteLength;
  }
  const headers = new Headers(request.headers);
  headers.delete('Transfer-Encoding');
  headers.set('Content-Length', String(length));
  return new Request(request.url, { method: request.method, headers, body: bytes });
}
export function createGateway({ authorize, fetcher = fetch, now = Date.now } = {}) {
  const verifier = new OidcVerifier(fetcher, now),
    local = new Map();
  let boxVerifier, boxSecret;
  let boxWindow = -1;
  const boxCounts = new Map();
  let controlWindow = -1,
    controlCount = 0;
  const controlIps = new Map();
  async function controlLimit(request, env, path) {
    if (!/^\/(?:status|logs|stop)\//.test(path)) return;
    const bearer = request.headers.get('Authorization')?.replace(/^Bearer /, '');
    if (
      path.startsWith('/stop/') &&
      validSecret(env.OWNER_STOP_TOKEN) &&
      (await constantTimeEqual(env.OWNER_STOP_TOKEN, bearer))
    )
      return;
    const cfg = configuration(env),
      window = Math.floor(now() / 60000);
    if (window !== controlWindow) {
      controlWindow = window;
      controlCount = 0;
      controlIps.clear();
    }
    const ip = request.headers.get('CF-Connecting-IP') ?? 'unknown';
    const count = controlIps.get(ip) ?? 0;
    if (
      count >= cfg.controlIp ||
      controlCount >= cfg.controlGlobal ||
      (!controlIps.has(ip) && controlIps.size >= DEFAULTS.controlMaxIps)
    )
      refuse('RATE_LIMITED', 429);
    controlIps.set(ip, count + 1);
    controlCount++;
  }
  async function frontAuthorize(request, env, path) {
    const cfg = configuration(env),
      bearer = request.headers.get('Authorization')?.replace(/^Bearer /, '');
    if (path === '/rent' && env.RENTING_ENABLED !== 'true') return;
    if (!bearer) refuse('UNAUTHORIZED', 401);
    if (
      (path === '/stop-all' || /^\/stop\//.test(path)) &&
      validSecret(env.OWNER_STOP_TOKEN) &&
      (await constantTimeEqual(env.OWNER_STOP_TOKEN, bearer))
    )
      return;
    if (/^\/(?:report|kill|blob-url)\//.test(path)) {
      if (boxSecret !== env.RUN_TOKEN_SECRET) {
        boxSecret = env.RUN_TOKEN_SECRET;
        boxVerifier = new RunTokens(boxSecret, now);
      }
      const fields = await boxVerifier.verifyStateless(bearer);
      const key = /^\/[^/]+\/([1-9][0-9]*-[1-9][0-9]*)/.exec(path)?.[1];
      if (!fields || fields.key !== key) refuse('UNAUTHORIZED', 401);
      const window = Math.floor(now() / 60000);
      if (boxWindow !== window) {
        boxWindow = window;
        boxCounts.clear();
      }
      const used = boxCounts.get(key) ?? 0;
      if (used >= 300 || (!boxCounts.has(key) && boxCounts.size >= 1024))
        refuse('RATE_LIMITED', 429);
      boxCounts.set(key, used + 1);
      return;
    }
    const identity = localIdentity(env, request.url);
    let selected = verifier;
    if (identity) {
      selected = local.get(identity.issuer);
      if (!selected) {
        if (local.size >= 4) local.clear();
        selected = new OidcVerifier(fetcher, now, identity);
        local.set(identity.issuer, selected);
      }
    }
    verifiedIdentity(await selected.verify(bearer, cfg.audience), cfg);
  }
  return {
    async fetch(request, env) {
      let path;
      try {
        if (new URL(request.url).pathname.endsWith('/availability') && secretFormatError(env)) {
          const bearer = request.headers.get('Authorization')?.replace(/^Bearer /, '');
          const cfg = configuration(env);
          const c = verifiedIdentity(await verifier.verify(bearer, env.WORKER_URL), cfg);
          if (c.actor_id === env.OWNER_GITHUB_ID) refuse(secretFormatError(env), 503);
        }
      } catch (error) {
        return Response.json(
          { state: error instanceof BrokerError ? error.state : 'UNAVAILABLE' },
          { status: error instanceof BrokerError ? error.status : 503 },
        );
      }
      try {
        path = await routePath(request, env);
      } catch (error) {
        return Response.json(
          { state: error.state ?? 'UNAVAILABLE' },
          { status: error.status ?? 503 },
        );
      }
      if (path === null || !routeKind(path)) return notFound();
      let buffered;
      try {
        await (authorize ?? frontAuthorize)(request, env, path);
        await controlLimit(request, env, path);
        buffered = await detachedUpload(request, path);
      } catch (error) {
        return Response.json(
          {
            state: error instanceof BrokerError ? error.state : 'UNAVAILABLE',
            verdict: 'UNAVAILABLE',
            log_tail_sanitised: '',
            bytes_dropped: 0,
          },
          {
            status: error instanceof BrokerError ? error.status : 503,
            headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
          },
        );
      }
      return env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(buffered);
    },
    scheduled(_event, env, ctx) {
      ctx.waitUntil(
        env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account')).fetch(
          new Request('https://internal.invalid/cron', { method: 'POST' }),
        ),
      );
    },
  };
}
export default createGateway();
