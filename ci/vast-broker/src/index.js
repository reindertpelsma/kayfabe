import { applyProfile, tierEnvironment, tierForClaims, familyOf, profileCaps } from './tier.js';
// prettier-ignore
import {planRelease,classifyLeg,releaseVerdict,wantsRerun} from './release.js';
// prettier-ignore
import {DEFAULTS} from './defaults.js';
import {
  BrokerError,
  budgetClass,
  refuse,
  budgetRefuse,
  configuration,
  teardownConfiguration,
  numericId,
  hexSha,
  localIdentity,
  readBounded,
} from './policy.js';
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
// prettier-ignore
import {benchObject,benchBudget,benchCount,benchExpiry,benchUrl,benchRedirect} from './benchdata.js';
import { offerPrice, providerId } from './policy.js';
import { reportedProviderFault } from './provider-fault.js';
// prettier-ignore
import {EVIDENCE_MAX_BYTES,evidenceEnabled,evidenceLength,storeEvidence} from './evidence.js';
// prettier-ignore
import {Vast} from './vast.js';
// prettier-ignore
import {OfferBook} from './offers.js';
// prettier-ignore
import {Ledger,rentalCost,costAttempt,spendClass} from './ledger.js';
// prettier-ignore
import {onstart} from './onstart.js';
// prettier-ignore
import {appendBoxText,appendWorkerText,sanitizeBoxText} from './sanitize.js';
// prettier-ignore
import {CHECK_PATH,storeEnabled,enqueue,freeze,storeTick,storeTrees,githubTreeKey,checkRequirements} from './ingest.js';
// Signed facts only: repository, environment subject, master ref, run identity and a pinned
// master workflow. Who triggered the run and which commit it tests come from one run lookup.
// Log-only trace of a 401: the route word and the fixed check code, never a value.
function logRefusal(error, path) {
  if (error instanceof BrokerError && error.status === 401)
    console.log(
      JSON.stringify({
        event: 'refused',
        route: typeof path === 'string' ? (path.split('/')[1] ?? '') : '',
        status: 401,
        why: typeof error.why === 'string' ? error.why : 'other',
      }),
    );
}
function identityFailure(c, cfg) {
  const checks = [
    ['repository', c.repository === cfg.repo],
    ['repository_id', c.repository_id === cfg.repositoryId],
    ['repository_owner_id', c.repository_owner_id === cfg.owner],
    ['sub', c.sub === cfg.oidcSubject],
    ['ref', c.ref === 'refs/heads/master'],
    ['actor_id', numericId(c.actor_id)],
    ['run_id', numericId(c.run_id)],
    ['run_attempt', numericId(c.run_attempt)],
    ['sha', hexSha(c.sha)],
    ['workflow', allowedWorkflowRef(c, cfg)],
  ];
  return checks.find(([, ok]) => !ok)?.[0];
}
function verifiedIdentity(c, cfg) {
  const failed = identityFailure(c, cfg);
  if (failed) refuse('UNAUTHORIZED', 401, undefined, 'identity:' + failed);
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
    admissionCount = 0,
    checkWindow = -1,
    checkCount = 0;
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
    env.R2_READ_ACCESS_KEY_ID,
    env.R2_READ_SECRET_ACCESS_KEY,
    env.ROW_TAG_KEY,
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
    instance_id: r.id,
    geolocation: r.geolocation ?? 'unknown',
    dph_total: r.dphTotal ?? null,
    host_driver_version: r.hostDriverVersion ?? 'unknown',
    host_reference: r.result?.host_reference ?? 'MISSING',
    provider_retries: r.providerRetries ?? 0,
    sha: r.sha,
    state: phase(r),
    verdict:
      r.ended !== null || r.resultAt !== undefined ? r.verdict || 'UNAVAILABLE' : 'UNAVAILABLE',
    cleanup_pending: r.ended === null,
    result_at: r.resultAt ?? null,
    ended_at: r.ended,
    started_at: r.originalStarted ?? r.started,
    deadline_at: Number.isFinite(nextDeadline(r)) ? nextDeadline(r) : r.started,
    gpu_model: r.hardware?.gpu_name ?? r.gpu ?? 'unknown',
    cost_estimate_usd:
      ((r.computeStoragePrice ?? r.price) * ((r.ended ?? now()) - r.started)) / 3600000 +
      (r.bandwidthCost ?? 0) +
      (r.retryCostUsd ?? 0),
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
    // Guest driver and the public offer-side host label are separate diagnostics.
    driver_version: r.hardware?.driver_version ?? 'unknown',
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
    const cap = profileCaps(cfg, budgetClass(record)).price;
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
      if (record.providerRetries === 1 && record.retryCostUsd + total > record.runBudgetUsd) {
        record.computeStoragePrice = computeStorage;
        record.bandwidthCost = bandwidth;
        await close(record, vast, 'CONTRACT_BUDGET', 'PROVIDER_FAULT');
        budgetRefuse('run-usd', {
          cost_usd: total,
          prior_usd: record.retryCostUsd,
          run_cap_usd: record.runBudgetUsd,
        });
      }
      // Review F8: a release leg's verified contract price is re-checked against its release's cost
      // cap (the other legs' spend and reservations plus this leg's full liability).
      if (budgetClass(record) === 'release' && typeof record.batch === 'string') {
        const batch = await ledger.get('batch:' + record.batch),
          others = batch ? await batchSpend(batch, record.key) : 0;
        if (!batch || !(others + total <= batch.costCapUsd)) {
          record.computeStoragePrice = computeStorage;
          record.bandwidthCost = bandwidth;
          await close(record, vast, 'CONTRACT_BUDGET', 'UNAVAILABLE');
          budgetRefuse('run-usd', {
            cost_usd: total,
            prior_usd: others,
            run_cap_usd: batch?.costCapUsd ?? 0,
          });
        }
      }
      try {
        const credit = await vast.credit();
        await ledger.budgets(cfg, now(), total, record.key, { credit, sameAttempt: true });
      } catch (error) {
        await close(record, vast, 'CONTRACT_BUDGET', 'UNAVAILABLE');
        if (error instanceof BrokerError && error.state === 'UNAVAILABLE_BUDGET') throw error;
        budgetRefuse('credit-unavailable', { cost_usd: total, reserve_usd: cfg.reserve });
      }
      record.computeStoragePrice = computeStorage;
      record.bandwidthCost = bandwidth;
      record.price = computeStorage + bandwidth / (cfg.hard / 3600000);
      await ledger.save(record);
      await ledger.put('cost:' + record.key, {
        ...costAttempt(record),
        ...spendClass(record),
        started: record.started,
        ended: null,
        ttl,
        created: true,
        price: computeStorage,
        bandwidthCost: bandwidth,
      });
    }
  }
  async function update(record, vast, github, cfg) {
    applyProfile(cfg, budgetClass(record));
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
        ['created', 'running'].includes(record.providerState) &&
        record.bootFailure?.confirmed &&
        now() - record.bootFailure.last <= cfg.cronPeriod * 2 &&
        Number.isSafeInteger(record.machineId)
      ) {
        await ledger.markProviderFault(record, 'KVM_DOMAIN_NOT_FOUND', now());
        await workerLine(record, 'host excluded after frozen KVM boot signature', cfg);
      }
      await workerLine(record, 'destroy reason=' + reason, cfg);
      const providerFault =
        reason === 'BOOT_TIMEOUT' &&
        ['created', 'running'].includes(record.providerState) &&
        record.bootFailure?.confirmed &&
        now() - record.bootFailure.last <= cfg.cronPeriod * 2;
      await close(
        record,
        vast,
        providerFault ? 'PROVIDER_FAULT' : reason,
        providerFault ? 'PROVIDER_FAULT' : reason === 'BOOT_TIMEOUT' ? 'UNAVAILABLE' : 'FAIL',
      );
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
    if (
      ['created', 'running'].includes(instance.actual_status) &&
      record.firstReportAt === undefined
    ) {
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
    // An admin or release leg has no GitHub run: its hard, boot and heartbeat deadlines bound it.
    if (record.source === 'admin') return;
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
        const providerReason = reportedProviderFault(r.result, r.log);
        if (providerReason) r.result.verdict = 'PROVIDER_FAULT';
        if (r.profile === 'sweep' && payload.result.verdict === 'PASS')
          await ledger.put('passed:' + r.die, now());
        r.resultAt = now();
        r.verdict = r.result.verdict;
        r.grace = r.verdict === 'PASS' ? cfg.passGrace : cfg.failGrace;
        r.resultDeadline = r.resultAt + r.grace;
        r.phase = 'GRACE';
        r.liveCheckedAt = null;
        if (r.verdict === 'PROVIDER_FAULT') {
          await ledger.markProviderFault(r, 'PROVIDER_FAULT', now());
          await workerLine(
            r,
            'provider fault: machine ' +
              r.machineId +
              ' host ' +
              (r.hostId ?? 'unknown') +
              ', not a kayfabe verdict',
            cfg,
          );
          await close(r, vast, 'PROVIDER_FAULT', 'PROVIDER_FAULT');
        }
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
    const bench =
      /^\/bench-data\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})\/([A-Za-z0-9._-]{1,100})$/.exec(path);
    const match = /^\/(report|kill|status|stop|logs)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.exec(
      path,
    );
    if (
      !(blob && request.method === 'GET') &&
      !(bench && request.method === 'GET') &&
      !(path === '/availability' && request.method === 'GET') &&
      !(path === '/rent' && request.method === 'POST') &&
      !(path === '/stop-all' && request.method === 'POST') &&
      !(path === CHECK_PATH && request.method === 'POST') &&
      !(match && request.method === (['status', 'logs'].includes(match[1]) ? 'GET' : 'POST'))
    )
      return notFound();
    const authorization = request.headers.get('Authorization');
    if (!authorization?.startsWith('Bearer ')) refuse('UNAUTHORIZED', 401);
    const bearer = authorization.slice(7),
      vast = new Vast(fetcher, cfg, env.VAST_API_KEY);
    if (bench) {
      const r = await ledger.get('record:' + bench[1]);
      if (
        !r ||
        r.key !== bench[1] ||
        !(await tokens.verify(bearer, r)) ||
        r.ended !== null ||
        r.closed ||
        now() >= r.reportExpiry * 1000
      )
        refuse('UNAUTHORIZED', 401);
      clocks(r, cfg);
      await rejectExpiredCallback(r, cfg, vast);
      await readBounded(request, 0);
      const objectKey = benchObject(bench[2]),
        used = benchBudget(r, bench[2]);
      await rate(r, cfg);
      const instance = await vast.inspect(r);
      if (!instance || instance.actual_status !== 'running') refuse('UNAUTHORIZED', 401);
      const at = now(),
        expires = benchExpiry(at, nextDeadline(r)),
        location = await benchUrl(env, objectKey, expires, at);
      benchCount(r, bench[2], used);
      await ledger.save(r);
      console.log(
        JSON.stringify({
          event: 'bench_data_url',
          run: bench[1],
          object: bench[2],
          expires: Math.floor(at / 1000) + expires,
        }),
      );
      return benchRedirect(location);
    }
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
    let checkBody;
    if (path === CHECK_PATH) checkBody = await readBounded(request, 1024);
    else if (path !== '/rent' && request.body !== null) {
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
      // No batch may start another leg after an owner stop-all.
      for (const [, id] of await ledger.storage.list({ prefix: 'batch-open:', limit: 257 })) {
        const batch = await ledger.get('batch:' + id);
        if (batch?.state !== 'RUNNING') continue;
        batch.state = 'STOPPED';
        for (const l of batch.legs) if (l.state === 'PENDING') l.state = 'MISSING';
        await ledger.put('batch:' + id, batch);
      }
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
    // The result check has its own admission counter: a polling gate never starves /rent.
    if (path === CHECK_PATH) {
      const window = Math.floor(now() / 60000);
      if (checkWindow !== window) {
        checkWindow = window;
        checkCount = 0;
      }
      if (++checkCount > DEFAULTS.admissionPerMinute) refuse('RATE_LIMIT', 429);
    }
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
        // runner_nonce, tested_sha, profile and remaining_seconds (the runner's remaining wait
        // budget, review B1) are required; preferences is optional.
        const allowed = [
            'runner_nonce',
            'tested_sha',
            'profile',
            'remaining_seconds',
            'preferences',
          ],
          keys = Object.keys(rentRequest),
          matches = [
            ...text.matchAll(
              /"(runner_nonce|tested_sha|profile|remaining_seconds|preferences)"\s*:/g,
            ),
          ];
        if (
          keys.length < 4 ||
          keys.length > 5 ||
          !keys.every((k) => allowed.includes(k)) ||
          !allowed.slice(0, 4).every((k) => keys.includes(k)) ||
          matches.length !== keys.length ||
          new Set(matches.map((x) => x[1])).size !== keys.length ||
          !/^([0-9a-f]{64})$/.test(rentRequest.runner_nonce) ||
          !/^([0-9a-f]{40})$/.test(rentRequest.tested_sha) ||
          !Number.isSafeInteger(rentRequest.remaining_seconds) ||
          rentRequest.remaining_seconds < 0 ||
          rentRequest.remaining_seconds > 86400
        )
          throw Error();
      } catch {
        refuse('BAD_REQUEST', 400);
      }
    }
    const token = env.GITHUB_API_TOKEN;
    if (token && (token.length > 1024 || !/^[\x21-\x7e]+$/.test(token))) refuse();
    const github = new Github(fetcher, cfg, token, { cache: githubCache, now });
    // Read-only result check (src/ingest.js): never reserves, rents or queues. It runs outside
    // the serial queue (fetch below), so its GitHub reads never hold up box reports.
    if (path === CHECK_PATH) {
      await github.authorize(c);
      return json(
        await checkRequirements({
          bytes: checkBody,
          env,
          github,
          ledger,
          treeKey: githubTreeKey(github),
          now: now(),
        }),
      );
    }
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
    if (!allowedWorkflowRef(c, cfg, profile))
      refuse('UNAUTHORIZED', 401, undefined, 'identity:profile_workflow');
    applyProfile(cfg, profile);
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
    let retrySource;
    if (prior) {
      if (prior.runnerNonce !== rentRequest.runner_nonce) refuse('BINDING_MISMATCH', 409);
      await recover(prior, vast);
      if (prior.verdict === 'PROVIDER_FAULT' && prior.closed && prior.ended === null)
        await cleanup(prior, vast);
      if (
        prior.verdict === 'PROVIDER_FAULT' &&
        prior.closed === 'PROVIDER_FAULT' &&
        prior.ended !== null &&
        prior.id &&
        (prior.providerRetries ?? 0) === 0 &&
        Number.isFinite(prior.runBudgetUsd) &&
        now() < prior.softDeadline
      ) {
        retrySource = prior;
        cfg.hard = prior.hardDeadline - now();
        cfg.boot = Math.min(cfg.boot, cfg.hard);
      } else {
        if (prior.id && prior.ended === null)
          await checkContract(prior, await vast.inspect(prior), vast, cfg);
        return json(expose(prior, key));
      }
    }
    // The tier comes from the verified event (src/tier.js), never from the request body.
    const admitted = await admit({
      key,
      target,
      profile,
      tierName: tierForClaims(c, profile),
      cfg,
      vast,
      github,
      preferences: rentRequest.preferences,
      runnerNonce: rentRequest.runner_nonce,
      runnerSeconds: rentRequest.remaining_seconds,
      force: Number(c.run_attempt) > 1, // An owner re-run; the SHA gap below still applies.
      retrySource,
    });
    return json(expose(admitted ?? retrySource, key));
  }
  // One admission path for every rental: the GitHub-triggered check and sweep, the owner's admin
  // start and each leg of a release. Every check below applies to all of them; the only
  // differences are inputs: a GitHub run brings its runner's remaining wait budget and may carry a
  // provider-fault replacement; a batch leg (admin start or release) never names a GitHub run, and
  // only a release leg is exempt from the one-rental-per-commit gap inside its own release (the
  // release holds that gap, and the release cost cap bounds its legs). The results store records
  // every rental admitted here from the ledger; records carry `tier`, `family`, `source`, `batch`.
  async function admit({
    key,
    target,
    profile,
    tierName,
    cfg,
    vast,
    github,
    preferences,
    runnerNonce,
    runnerSeconds,
    force = false,
    retrySource,
    leg,
    market,
    costGuard,
  }) {
    // Review F1: the switches are checked here, on the one admission path, so turning renting (or
    // the release kill switch) off also stops cron-started release legs and re-runs.
    if (env.RENTING_ENABLED !== 'true') refuse('RENTING_DISABLED', 503);
    if (profile === 'release' && !cfg.releaseEnabled) refuse('RELEASE_DISABLED', 503);
    // Review R1: one attempt per run key, also after retention removed the record. Guaranteed:
    // while the spend row exists the key is refused. The row is pruned once its run ended more
    // than 30 days ago (ledger.budgets), so after that a token minted for the same run attempt
    // (owner only; GitHub allows a run up to 35 days) could rent again. Not guaranteed beyond.
    const previousCost = await ledger.get('cost:' + key);
    if (previousCost && !retrySource)
      budgetRefuse('cost-key', { started_at: previousCost.started, ended_at: previousCost.ended });
    // Review B2: the commit gap before the cap, so a repeated commit is refused at once instead
    // of waiting BUSY, and before the supersede, so it closes nobody.
    const cached = await ledger.get('sha:' + target.sha);
    // A fresh runner nonce may not inherit another run's result.
    if (
      !retrySource &&
      !(leg && (await ledger.batchLeg(leg.batch, key, target.sha))) &&
      cached &&
      (!force || now() - cached.started < cfg.gap)
    )
      budgetRefuse('sha-gap', { gap_ms: cfg.gap, elapsed_ms: Math.max(0, now() - cached.started) });
    // Review B1: a run whose remaining budget cannot hold the hard limit plus the margin is not
    // admitted (it would time out with a paid box) and supersedes nothing.
    if (
      runnerSeconds !== undefined &&
      runnerSeconds * 1000 < cfg.hard + DEFAULTS.admissionMarginMinutes * 60000
    )
      refuse('NOT_ELIGIBLE', 409, 'RUNNER_TIME');
    // Owner ruling 2026-10-24 item 8: a set of active runs (every record not yet ended). A newer
    // run supersedes only the older runs of its own branch; an older run's late request never
    // closes a newer one and is not eligible. Over the cap a run gets BUSY and retries.
    const open = await ledger.openRecords(),
      branch = open.filter((o) => o.branch === target.branch && o.sha !== target.sha);
    if (branch.some((o) => numericId(o.run) && BigInt(o.run) > BigInt(target.run)))
      refuse('NOT_ELIGIBLE', 409);
    if (branch.length) {
      for (const o of branch) await close(o, vast, 'SUPERSEDED');
      refuse('BUSY', 409);
    }
    if (
      open.length >= cfg.maxConcurrent ||
      open.filter((o) => budgetClass(o) === profile).length >= cfg.maxConcurrentProfile[profile]
    )
      refuse('BUSY', 409);
    if (!validSecret(env.RUN_TOKEN_SECRET) || !validSecret(env.OWNER_STOP_TOKEN)) refuse();
    await github.boxProtocol(target.sha);
    const book = retrySource || market !== undefined ? new OfferBook(now) : offers;
    const evaluation = await book.evaluate(vast, cfg, ledger, profile, {
      market,
      vramFloorMb: tierName === 'middle' ? cfg.middleVramFloorMb : 0,
    });
    if (retrySource) {
      const remaining = retrySource.runBudgetUsd - rentalCost(retrySource, retrySource.ended);
      evaluation.allowed = evaluation.allowed.filter(
        (offer) =>
          offer.machine_id !== retrySource.machineId &&
          providerId(retrySource.hostId) !== null &&
          providerId(offer.host_id) !== null &&
          offer.host_id !== retrySource.hostId &&
          offer.pricing.total <= remaining,
      );
      if (!evaluation.allowed.length) return null;
    }
    const chosen = await offers.select(evaluation, cfg, ledger, profile, preferences, {
        tierName,
        ...(leg?.die ? { die: leg.die } : {}),
        avoidMachines: leg?.avoidMachines ?? [],
        avoidHosts: leg?.avoidHosts ?? [],
      }),
      offer = chosen.offer,
      cost = offer.pricing.total,
      credit = await vast.credit();
    try {
      if (costGuard) costGuard(cost);
      await ledger.budgets(cfg, now(), cost, null, {
        credit,
        sameAttempt: retrySource !== undefined,
      });
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
        ...spendClass({ profile }),
        tier: tierName,
        family: chosen.family ?? familyOf(offer.die, cfg),
        ...(chosen.wanted !== undefined && chosen.wanted !== chosen.family
          ? { familyWanted: chosen.wanted }
          : {}),
        ...(leg
          ? {
              batch: leg.batch,
              source: 'admin',
              windowsRuns: leg.windowsRuns,
              stages: leg.stages,
            }
          : { source: 'github' }),
        die: offer.die,
        country: offer.country,
        geolocation:
          typeof offer.geolocation === 'string'
            ? offer.geolocation.replace(/[\p{Cc}\p{Cf}]/gu, ' ').slice(0, 128)
            : 'unknown',
        dphTotal: offer.dph_total,
        hostDriverVersion:
          typeof offer.driver_version === 'string' &&
          /^(?:unknown|\d{3}\.\d{1,3}(?:\.\d{1,2})?)$/.test(offer.driver_version)
            ? offer.driver_version
            : 'unknown',
        computeCap: offer.compute_cap,
        machineId: offer.machine_id,
        hostId: providerId(offer.host_id),
        preferencesIgnored: chosen.ignored,
        selectionReason: chosen.reason,
        bandwidthCost: offer.pricing.bandwidth,
        computeStoragePrice: offer.pricing.computeStorage,
        key,
        label: 'kf-ci-' + key,
        nonce: n,
        runnerNonce,
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
    r.runBudgetUsd = retrySource?.runBudgetUsd ?? profileCaps(cfg, profile).dailyUsd;
    r.providerRetries = retrySource ? 1 : 0;
    if (retrySource) {
      r.originalStarted = retrySource.started;
      r.hardDeadline = retrySource.hardDeadline;
      r.softDeadline = retrySource.softDeadline;
      r.bootDeadline = Math.min(r.bootDeadline, r.softDeadline);
      r.ttl = r.hardDeadline - r.started;
      r.billingTtl = r.ttl + cfg.cronPeriod;
      r.reportExpiry = Math.floor(r.hardDeadline / 1000);
      r.retryCostUsd = rentalCost(retrySource, retrySource.ended);
      r.nonce = (await sha256Hex(retrySource.nonce + n + ':provider:1')).slice(0, 32);
      // Keep one authenticated log chain and the run-wide byte/report limits across legs.
      for (const field of [
        'log',
        'logChunks',
        'logHash',
        'fullBytes',
        'fullBytesDropped',
        'bytesDropped',
        'reportBytes',
        'reportCount',
      ])
        if (retrySource[field] !== undefined) r[field] = retrySource[field];
    }
    const runToken = await tokens.issue(r),
      script = onstart(target.sha, cfg.repo);
    r.scriptHash = await sha256Hex(script);
    try {
      await ledger.reserve(r, { cfg, now: now(), cost, credit, retryNonce: retrySource?.nonce });
    } catch (e) {
      if (e instanceof BrokerError && e.state === 'UNAVAILABLE_BUDGET')
        e.selection = { die: offer.die, ignored: chosen.ignored, reason: chosen.reason };
      throw e;
    }
    // Result store queue (src/ingest.js); never affects admission or the rental.
    if (storeEnabled(env))
      try {
        if (retrySource) await freeze(ledger, retrySource);
        await enqueue(ledger, r);
      } catch {
        console.log(JSON.stringify({ event: 'store', state: 'QUEUE_FAILED' }));
      }
    await workerLine(
      r,
      'selection profile=' +
        profile +
        ' die=' +
        r.die +
        ' reason=' +
        r.selectionReason +
        ' ignored=' +
        r.preferencesIgnored +
        ' tier=' +
        tierName +
        ' family=' +
        r.family +
        (r.familyWanted ? ' fallback_from=' + r.familyWanted : ''),
      cfg,
    );
    if (profile === 'sweep') await ledger.put('rotation:lastDie', r.die);
    if (chosen.reason === 'ROTATE' && tierName === 'middle')
      await ledger.put('rotation:middleFamily', chosen.family);
    try {
      const environment = {
        KF_PROTOCOL: DEFAULTS.boxProtocol,
        KF_BROKER_URL: cfg.audience + '/' + env.BROKER_PATH_PREFIX,
        KF_RUN_ID: key,
        KF_RUN_TOKEN: runToken,
        KF_NONCE: r.nonce,
        KF_SHA: r.sha,
        KF_REPO: cfg.repo,
        KF_RUNNER_NONCE: r.runnerNonce,
        KF_BOOT_DEADLINE: Math.floor(r.bootDeadline / 1000),
        KF_HARD_DEADLINE: Math.floor(r.hardDeadline / 1000),
        KF_SOFT_DEADLINE: Math.floor(r.softDeadline / 1000),
        KF_HEARTBEAT_SECONDS: cfg.heartbeat / 1000,
        KF_GRACE_PASS: cfg.passGrace / 1000,
        KF_GRACE_FAIL: cfg.failGrace / 1000,
        // The tier: a pure function of the profile and the verified event or admin batch
        // (src/tier.js), never a request field.
        ...tierEnvironment(profile, {
          tierName,
          windowsRuns: leg?.windowsRuns ?? 1,
          stages: leg?.stages ?? [],
        }),
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
      if (e instanceof BrokerError && e.state === 'UNAVAILABLE_BUDGET') throw e;
      refuse();
    }
    r.phase = 'BOOTING';
    await ledger.save(r);
    console.log(JSON.stringify({ event: 'rented' }));
    return r;
  }
  // ---- Admin starts and the release (large) tier: batches (2026-11-05, wtier) ----------------
  // A batch is one owner start: a single leg for small or middle, the planned legs of a release.
  // Its legs are ordinary rentals through admit(); the cron tick classifies ended legs, schedules
  // a fresh-box re-run for RETRY/UNATTRIBUTED within the release cost cap, starts pending legs as
  // slots and caps allow, and finishes with a verdict over (family, die, platform) cells.
  const BATCH_KEYS = ['commit_sha', 'die', 'tier'];
  function batchRequest(body) {
    if (
      !body ||
      typeof body !== 'object' ||
      Array.isArray(body) ||
      !Object.keys(body).every((k) => BATCH_KEYS.includes(k)) ||
      !hexSha(body.commit_sha) ||
      !['small', 'middle', 'large'].includes(body.tier) ||
      (body.die !== undefined &&
        (body.tier === 'large' ||
          typeof body.die !== 'string' ||
          !/^[A-Z][A-Z0-9_]{1,23}$/.test(body.die)))
    )
      refuse('BAD_REQUEST', 400);
    return body;
  }
  function batchConfig(tierName) {
    const cfg = configuration(env);
    applyProfile(cfg, tierName === 'large' ? 'release' : 'check');
    return cfg;
  }
  const tierDies = (cfg, tierName) =>
    tierName === 'middle' ? Object.values(cfg.middleRotation).flat() : cfg.checkDies;
  async function releaseMarket(vast, cfg) {
    const seen = new Map();
    for (const cc of Object.values(cfg.families))
      for (const offer of await vast.offers({ computeCap: cc, maxPrice: cfg.releasePrice }))
        if (!seen.has(offer.id)) seen.set(offer.id, offer);
    return [...seen.values()];
  }
  async function planFor(request, cfg, vast) {
    if (request.tier === 'large') {
      const market = await releaseMarket(vast, cfg);
      const evaluation = await new OfferBook(now).evaluate(vast, cfg, ledger, 'release', {
        market,
      });
      return planRelease(evaluation.allowed, cfg, {
        budget: await ledger.budgetSummary(cfg, now()),
        credit: await vast.credit(),
      });
    }
    if (request.die !== undefined && !tierDies(cfg, request.tier).includes(request.die))
      refuse('BAD_REQUEST', 400, 'DIE_NOT_IN_TIER');
    const evaluation = await new OfferBook(now).evaluate(vast, cfg, ledger, 'check', {
      market: await vast.offers(),
      vramFloorMb: request.tier === 'middle' ? cfg.middleVramFloorMb : 0,
    });
    let chosen = null,
      refusal = null;
    try {
      chosen = await offers.select(evaluation, cfg, ledger, 'check', undefined, {
        tierName: request.tier,
        ...(request.die ? { die: request.die } : {}),
      });
    } catch (e) {
      refusal = e instanceof BrokerError ? (e.reason ?? e.state) : 'UNAVAILABLE';
    }
    return {
      tier: request.tier,
      profile: 'check',
      hard_minutes: cfg.hard / 60000,
      price_cap_usd_hour: cfg.price,
      vram_floor_mb: request.tier === 'middle' ? cfg.middleVramFloorMb : 0,
      legs: chosen ? 1 : 0,
      die: chosen?.offer.die ?? null,
      family: chosen ? (chosen.family ?? familyOf(chosen.offer.die, cfg)) : null,
      machine_id: chosen ? providerId(chosen.offer.machine_id) : null,
      estimate_usd: chosen ? Math.round(chosen.offer.pricing.total * 10000) / 10000 : 0,
      admitted: chosen !== null,
      refusals: refusal ? [refusal] : [],
    };
  }
  async function adminPlan(body) {
    const request = batchRequest(body),
      cfg = batchConfig(request.tier),
      vast = new Vast(fetcher, cfg, env.VAST_API_KEY);
    return { plan: await planFor(request, cfg, vast), renting: false };
  }
  async function adminStart(body) {
    if (env.RENTING_ENABLED !== 'true') refuse('RENTING_DISABLED', 503);
    const request = batchRequest(body),
      cfg = batchConfig(request.tier),
      vast = new Vast(fetcher, cfg, env.VAST_API_KEY),
      token = env.GITHUB_API_TOKEN;
    if (request.die !== undefined && !tierDies(cfg, request.tier).includes(request.die))
      refuse('BAD_REQUEST', 400, 'DIE_NOT_IN_TIER');
    if (request.tier === 'large' && !cfg.releaseEnabled) refuse('RELEASE_DISABLED', 503);
    if (token && (token.length > 1024 || !/^[\x21-\x7e]+$/.test(token))) refuse();
    const github = new Github(fetcher, cfg, token, { cache: githubCache, now });
    // Forks never trigger rentals: the commit must be on this repository's master or next.
    await github.reachable(request.commit_sha);
    await github.boxProtocol(request.commit_sha);
    for (const [, open] of await ledger.storage.list({ prefix: 'batch-open:', limit: 257 })) {
      const other = await ledger.get('batch:' + open);
      if (
        other?.state === 'RUNNING' &&
        other.sha === request.commit_sha &&
        other.tier === request.tier
      )
        refuse('BUSY', 409, 'BATCH_RUNNING');
    }
    const plan = await planFor(request, cfg, vast);
    if (!plan.admitted) {
      const error = new BrokerError('NOT_ELIGIBLE', 409, 'PLAN_REFUSED');
      error.plan = plan;
      throw error;
    }
    const seq = ((await ledger.get('batch-seq')) ?? 0) + 1;
    await ledger.put('batch-seq', seq);
    const id = '8' + String(now()).padStart(15, '0') + String(seq % 1000).padStart(3, '0');
    const legs = [];
    const leg = (family, die, windowsRuns) => {
      const n = legs.length + 1;
      legs.push({
        seq: n,
        key: id + '-' + n,
        family,
        die,
        windowsRuns,
        state: 'PENDING',
        avoidMachines: [],
        avoidHosts: [],
        rerunDepth: 0,
      });
    };
    if (request.tier === 'large') {
      for (const f of plan.families)
        for (const c of f.cells) for (const l of c.legs) leg(f.family, c.die, l.windows_runs);
    } else leg(request.die ? familyOf(request.die, cfg) : null, request.die ?? null, 1);
    const batch = {
      id,
      sha: request.commit_sha,
      tier: request.tier,
      profile: request.tier === 'large' ? 'release' : 'check',
      state: 'RUNNING',
      created: now(),
      updated: now(),
      runnerNonce: await sha256Hex(
        'kf-batch:' + id + ':' + (random ? random() : crypto.randomUUID()),
      ),
      costCapUsd: request.tier === 'large' ? cfg.releaseCostCap : null,
      estimateUsd: plan.estimate_usd,
      windowsRunsPerDie: request.tier === 'large' ? cfg.releaseWindowsRuns : 1,
      diesPerFamily: cfg.releaseDiesPerFamily,
      releaseDies: request.tier === 'large' ? cfg.releaseDies : {},
      stages: request.tier === 'large' ? cfg.releaseStages : [],
      reruns: request.tier === 'large' ? cfg.releaseReruns : 0,
      legs,
      verdict: null,
    };
    await ledger.put('batch:' + id, batch);
    await ledger.put('batch-open:' + id, id);
    console.log(JSON.stringify({ event: 'batch_started', tier: batch.tier, legs: legs.length }));
    await tickBatch(batch, vast, github);
    return batchView(batch, plan);
  }
  function batchView(batch, plan) {
    return {
      batch: batch.id,
      sha: batch.sha,
      tier: batch.tier,
      state: batch.state,
      cost_cap_usd: batch.costCapUsd,
      estimate_usd: batch.estimateUsd,
      spent_usd: batch.spentUsd ?? 0,
      verdict: batch.verdict,
      legs: batch.legs.map((l) => ({
        key: l.key,
        family: l.family,
        die: l.die,
        windows_runs: l.windowsRuns,
        state: l.state,
        verdict: l.outcome?.verdict ?? null,
        rerun_of: l.rerunOf ?? null,
      })),
      ...(plan ? { plan } : {}),
    };
  }
  // Spend of a batch: settled legs at their cost, open legs at their reserved liability.
  async function batchSpend(batch, exclude) {
    let spent = 0;
    for (const l of batch.legs) {
      if (l.key === exclude) continue;
      if (l.state === 'ENDED') spent += l.costUsd ?? 0;
      else if (l.state === 'STARTED') {
        const r = await ledger.get('record:' + l.key);
        if (r)
          spent +=
            ((r.computeStoragePrice ?? r.price) * (r.billingTtl ?? r.ttl)) / 3600000 +
            (r.bandwidthCost ?? 0);
      }
    }
    return spent;
  }
  async function tickBatch(batch, vast, github) {
    const cfg = batchConfig(batch.tier);
    // 1. Ended legs: classify once, then a fresh-box re-run for RETRY/UNATTRIBUTED if allowed.
    for (const l of [...batch.legs]) {
      if (l.state !== 'STARTED') continue;
      const r = await ledger.get('record:' + l.key);
      if (r && r.ended === null) continue;
      l.state = 'ENDED';
      // Only this batch's own run of the SAME commit counts (no tree-key carry-over, coordinator
      // ruling 2026-11-05): a record of another commit can never answer a release cell.
      l.outcome = classifyLeg(r?.sha === batch.sha && r?.batch === batch.id ? r : undefined, {
        windowsRuns: l.windowsRuns,
        stages: batch.stages,
        die: batch.tier === 'large' ? l.die : (l.die ?? r?.die),
        family: batch.tier === 'large' ? l.family : (l.family ?? r?.family),
        cfg,
      });
      l.costUsd = r ? rentalCost(r, r.ended) : 0;
      l.machineId = r?.machineId ?? null;
      if (batch.state === 'RUNNING' && wantsRerun(l, batch.reruns)) {
        const n = Math.max(...batch.legs.map((x) => x.seq)) + 1;
        batch.legs.push({
          seq: n,
          key: batch.id + '-' + n,
          family: l.family,
          die: l.die,
          windowsRuns: l.windowsRuns,
          state: 'PENDING',
          // Review F10: a fresh box: not a machine any leg of this die used, the failed one
          // included (its machineId is set above), so two Windows runs stay on two machines.
          avoidMachines: [
            ...new Set([
              ...l.avoidMachines,
              ...batch.legs.filter((x) => x.die === l.die && x.machineId).map((x) => x.machineId),
            ]),
          ],
          avoidHosts: [...l.avoidHosts, ...(r?.hostId ? [r.hostId] : [])],
          rerunDepth: (l.rerunDepth ?? 0) + 1,
          rerunOf: l.key,
          queued: now(),
        });
      }
    }
    // The ledger's batch-leg exemption reads the stored batch: store new re-run legs first.
    await ledger.put('batch:' + batch.id, batch);
    // 2. Pending legs: start while slots and caps allow; give up after the start window.
    let market;
    for (const l of batch.legs) {
      if (l.state !== 'PENDING') continue;
      if (
        batch.state !== 'RUNNING' ||
        now() - (l.queued ?? batch.created) > cfg.releaseStartWindow
      ) {
        l.state = 'MISSING';
        l.outcome = null;
        continue;
      }
      if (await ledger.get('record:' + l.key)) {
        l.state = 'STARTED';
        continue;
      }
      try {
        if (batch.tier === 'large') market ??= await releaseMarket(vast, cfg);
        const spent = await batchSpend(batch);
        const started = await admit({
          key: l.key,
          target: {
            sha: batch.sha,
            // Review F10: ':' cannot occur in a git ref, so no GitHub head branch shares it.
            branch: 'batch:' + batch.id,
            run: batch.id,
            attempt: String(l.seq),
          },
          profile: batch.profile,
          tierName: batch.tier,
          cfg,
          vast,
          github,
          preferences: undefined,
          runnerNonce: batch.runnerNonce,
          leg: {
            batch: batch.id,
            die: l.die ?? undefined,
            windowsRuns: l.windowsRuns,
            stages: batch.stages,
            avoidMachines: l.avoidMachines,
            avoidHosts: l.avoidHosts,
          },
          market,
          costGuard:
            batch.costCapUsd === null
              ? undefined
              : (cost) => {
                  if (spent + cost > batch.costCapUsd)
                    budgetRefuse('release-cost-cap', {
                      cost_usd: cost,
                      spent_usd: spent,
                      cap_usd: batch.costCapUsd,
                    });
                },
        });
        l.state = 'STARTED';
        delete l.lastRefusal;
        l.machineId = started.machineId;
        l.family ??= started.family;
        l.die ??= started.die;
        // A later leg of the same die (the second Windows run) belongs on another machine.
        for (const sibling of batch.legs)
          if (sibling.state === 'PENDING' && sibling.die === l.die && sibling !== l) {
            sibling.avoidMachines = [...new Set([...sibling.avoidMachines, started.machineId])];
            if (started.hostId)
              sibling.avoidHosts = [...new Set([...sibling.avoidHosts, started.hostId])];
          }
        // Durable at once (review F10): the next leg's contract check reads this leg's liability,
        // and a crash cannot lose the siblings' machine avoidance.
        await ledger.put('batch:' + batch.id, batch);
      } catch (e) {
        if (await ledger.get('record:' + l.key)) l.state = 'STARTED';
        const reason = e instanceof BrokerError ? (e.reason ?? e.state) : 'UNAVAILABLE';
        l.lastRefusal = reason;
        // Review F1/F4: admit() refused because renting or releases are switched off: nothing of
        // this batch is rented any more (its pending legs and re-runs end as MISSING).
        if (['RENTING_DISABLED', 'RELEASE_DISABLED'].includes(reason)) {
          for (const x of batch.legs) if (x.state === 'PENDING') x.state = 'MISSING';
          break;
        }
        // No offer for this die: try the next leg. Busy, a cap or anything else: next tick.
        if (reason !== 'NO_ALLOWED_OFFERS') break;
      }
    }
    // 3. The verdict; a batch with nothing pending or running is finished.
    batch.spentUsd = Math.round((await batchSpend(batch)) * 10000) / 10000;
    const view = releaseVerdict(batch);
    batch.verdict = view.verdict;
    batch.cells = view.cells;
    batch.updated = now();
    if (!batch.legs.some((l) => l.state === 'PENDING' || l.state === 'STARTED')) {
      if (batch.state === 'RUNNING') batch.state = 'DONE';
      await ledger.storage.delete('batch-open:' + batch.id);
    }
    await ledger.put('batch:' + batch.id, batch);
  }
  async function batchTicks() {
    const cfg = configuration(env);
    const vast = new Vast(fetcher, cfg, env.VAST_API_KEY),
      github = new Github(fetcher, cfg, env.GITHUB_API_TOKEN, { cache: githubCache, now });
    for (const [, id] of await ledger.storage.list({ prefix: 'batch-open:', limit: 64 })) {
      const batch = await ledger.get('batch:' + id);
      if (!batch) continue;
      try {
        await tickBatch(batch, vast, github);
      } catch {
        console.log(JSON.stringify({ event: 'batch_tick_unavailable' }));
      }
    }
    // Review F10: finished batches are kept 30 days (the spend rows' horizon), then pruned.
    let pruned = 0;
    for (const [key, batch] of await ledger.storage.list({ prefix: 'batch:', limit: 1000 })) {
      if (pruned >= 20) break;
      if (
        batch?.state !== 'RUNNING' &&
        Number.isFinite(batch?.updated) &&
        now() - batch.updated > 30 * 86400000
      ) {
        await ledger.storage.delete(key);
        pruned++;
      }
    }
  }
  // Owner stop of a whole batch: no new legs, every open leg closed by the existing stop path.
  async function stopBatches(ids, vast) {
    for (const id of ids) {
      const batch = await ledger.get('batch:' + id);
      if (!batch || batch.state !== 'RUNNING') continue;
      // Review F2: the stop is durable before any provider call, as in /stop-all; a failed close
      // leaves a STOPPED batch (no new leg) whose box the cron, the owner stop or the reaper ends.
      batch.state = 'STOPPED';
      for (const l of batch.legs) if (l.state === 'PENDING') l.state = 'MISSING';
      await ledger.put('batch:' + id, batch);
      let failed = false;
      for (const l of batch.legs) {
        if (l.state !== 'STARTED') continue;
        const r = await ledger.get('record:' + l.key);
        try {
          if (r && r.ended === null) await close(r, vast, 'STOPPED');
        } catch {
          failed = true;
        }
      }
      await tickBatch(batch, vast, null);
      if (failed) refuse();
    }
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
      /^\/(report|kill|evidence)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.exec(path) ??
      /^\/(blob-url)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})\/[A-Za-z0-9._-]{1,64}\/[A-Za-z0-9._-]{1,100}$/.exec(
        path,
      ) ??
      /^\/(bench-data)\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})\/[A-Za-z0-9._-]{1,100}$/.exec(path);
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
      ) &&
      path !== CHECK_PATH
    )
      return;
    await oidc(bearer, cfg);
  }
  // POST /evidence/<run>-<attempt> (src/evidence.js). One upload per rental: the ledger record
  // carries the state (PENDING while a body is being written, STORED after), checked and set in
  // the serial queue, so two uploads cannot both write. The queue is held only for those checks;
  // the body is read and written to R2 outside it. A failed write clears PENDING, so the box may
  // retry; a PENDING flag older than a minute (a crashed attempt) no longer blocks.
  const EVIDENCE_PENDING_MS = 60000;
  async function settleEvidence(key, at, stored, sha256 = null) {
    try {
      await serial(
        1,
        async () => {
          const r = await ledger.get('record:' + key);
          if (!r || r.evidence?.at !== at) return;
          if (stored) r.evidence = { state: 'STORED', at: now(), sha256 };
          else delete r.evidence;
          await ledger.save(r);
        },
        key,
      );
    } catch {
      // Best effort: a PENDING flag expires by itself.
    }
  }
  async function evidenceUpload(request, key) {
    if (request.method !== 'POST' || new URL(request.url).search) refuse('NOT_FOUND', 404);
    const bearer = request.headers.get('Authorization')?.replace(/^Bearer /, '');
    const cfg = configuration(env),
      vast = new Vast(fetcher, cfg, env.VAST_API_KEY);
    let at;
    const record = await serial(
      1,
      async () => {
        const r = await ledger.get('record:' + key);
        if (
          !r ||
          r.key !== key ||
          !(await tokens.verify(bearer, r)) ||
          r.ended !== null ||
          r.closed ||
          now() >= r.reportExpiry * 1000
        )
          refuse('UNAUTHORIZED', 401);
        if (!evidenceEnabled(env)) refuse('EVIDENCE_DISABLED', 404);
        clocks(r, cfg);
        await rejectExpiredCallback(r, cfg, vast);
        evidenceLength(request);
        await rate(r, cfg);
        const prior = r.evidence;
        if (prior && (prior.state === 'STORED' || now() - prior.at < EVIDENCE_PENDING_MS))
          refuse('EVIDENCE_EXISTS', 409);
        at = now();
        r.evidence = { state: 'PENDING', at };
        await ledger.save(r);
        return r;
      },
      key,
    );
    let answer;
    try {
      answer = await storeEvidence(env, record, request, now());
    } catch (error) {
      // An object that is already there (the head check) means stored, whoever put it.
      await settleEvidence(key, at, error instanceof BrokerError && error.status === 409);
      throw error;
    }
    await settleEvidence(key, at, true, answer.sha256);
    return answer;
  }
  return {
    async fetch(request) {
      let path;
      try {
        const receipt = await preauthenticate(request);
        if (receipt instanceof Response) return receipt;
        path = await routePath(request, env);
        // Outside the serial queue: it reads no ledger record and writes only treekey:<sha>.
        if (path === CHECK_PATH) return await perform(request);
        const evidence = /^\/evidence\/([1-9][0-9]{0,19}-[1-9][0-9]{0,19})$/.exec(path);
        if (evidence) return json(await evidenceUpload(request, evidence[1]));
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
        logRefusal(e, path);
        return json(
          {
            state: e instanceof BrokerError ? e.state : 'UNAVAILABLE',
            verdict: 'UNAVAILABLE',
            log_tail_sanitised: '',
            bytes_dropped: 0,
            ...(e instanceof BrokerError && e.reason ? { reason: e.reason } : {}),
            ...(e instanceof BrokerError && e.selection ? { selection: e.selection } : {}),
            ...(e instanceof BrokerError && e.budget ? { budget: e.budget } : {}),
          },
          e instanceof BrokerError ? e.status : 503,
        );
      } finally {
        await releaseUpload(request);
      }
    },
    // Tree keys for the store queue, OUTSIDE the serial queue (GitHub reads, bounded in time).
    async storeTrees() {
      if (!storeEnabled(env)) return;
      try {
        const github = new Github(fetcher, configuration(env), env.GITHUB_API_TOKEN, {
          cache: githubCache,
          now,
        });
        await storeTrees({ ledger, env, now: now(), treeKey: githubTreeKey(github) });
      } catch {
        console.log(JSON.stringify({ event: 'store', state: 'TREES_FAILED' }));
      }
    },
    async sweep() {
      return serial(0, async () => {
        try {
          // Review of PR #269, R3: a malformed policy var refuses every route, but never stops
          // teardown. The cron then closes and cleans up open rentals on the identity vars and the
          // committed defaults only, and skips everything that could start work (batch ticks) or
          // delete data on a default retention (expiry, the store).
          let cfg,
            teardownOnly = false;
          try {
            cfg = configuration(env);
          } catch (e) {
            if (!(e instanceof BrokerError)) throw e;
            try {
              cfg = teardownConfiguration(env);
            } catch {
              throw e;
            }
            teardownOnly = true;
            console.log(
              JSON.stringify({
                event: 'cleanup_config_error',
                state: /^CONFIG_[A-Z_]+:[A-Z_]+$/.test(e.state) ? e.state : 'CONFIG_UNNAMED',
                teardown: 'defaults',
              }),
            );
          }
          const records = await ledger.openRecords();
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
          if (teardownOnly) return;
          await batchTicks();
          await ledger.expire(now(), cfg);
          // After cleanup and expiry, bounded in time (src/ingest.js); tree keys come from the
          // cache that storeTrees fills outside this queue.
          if (storeEnabled(env))
            try {
              await storeTick({ ledger, env, now: now() });
            } catch {
              console.log(JSON.stringify({ event: 'store', state: 'TICK_FAILED' }));
            }
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
    // Owner admin routes (BrokerLedger, internal origin only): the same serialized ledger queue.
    adminPlan: (body) => serial(2, () => adminPlan(body), '@admin-plan'),
    adminStart: (body) => serial(2, () => adminStart(body), '@admin-start'),
    adminStopBatch: (id) =>
      serial(0, () => stopBatches([id], new Vast(fetcher, configuration(env), env.VAST_API_KEY))),
    adminBatches: async () => {
      const entries = await ledger.storage.list({ prefix: 'batch:', limit: 1000 });
      return [...entries.values()]
        .sort((a, b) => b.created - a.created)
        .slice(0, 50)
        .map((b) => batchView(b));
    },
  };
}
export class BrokerLedger {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
    this.service = createService({ env, storage: ctx.storage });
  }
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === '/admin/log') {
      if (url.origin !== 'https://internal.invalid' || request.method !== 'GET') return notFound();
      const json = (data, status = 200) =>
        Response.json(data, {
          status,
          headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
        });
      const params = url.searchParams,
        run = params.get('run'),
        raw = params.get('cursor') ?? '0';
      if (
        params.getAll('run').length !== 1 ||
        params.getAll('cursor').length > 1 ||
        [...params.keys()].some((key) => !['run', 'cursor'].includes(key)) ||
        !/^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(run ?? '') ||
        !/^(0|[1-9][0-9]{0,6})$/.test(raw)
      )
        return json({ error: 'Invalid log selection' }, 400);
      const cursor = Number(raw);
      try {
        const ledger = new Ledger(this.ctx.storage),
          record = await ledger.get('record:' + run);
        if (!record)
          return json({ text: 'log expired', summary: '', cursor, next_cursor: null }, 404);
        const count = record.logChunks ?? 0;
        if (!Number.isSafeInteger(count) || count < 0) throw Error();
        if (cursor > count) return json({ error: 'Invalid log selection' }, 400);
        // Reuse the relay reader, one chunk instead of the runner's default four.
        const page = await ledger.logPage(record, cursor, 1),
          tail = typeof record.log === 'string' ? record.log : '',
          encoder = new TextEncoder();
        if (
          encoder.encode(page.log_increment_sanitised).length > 16384 ||
          encoder.encode(tail).length > 8192
        )
          throw Error();
        const marker =
          String.raw`(?:^|\n)(?:\[box\] )?KF_FINAL_SUMMARY_BEGIN\b` +
          String.raw`[\s\S]*?KF_FINAL_SUMMARY_END[^\n]*(?:\n|$)`;
        const blocks = [...tail.matchAll(new RegExp(marker, 'g'))];
        return json({
          text: page.log_increment_sanitised,
          summary: blocks.at(-1)?.[0].replace(/^\n/, '') ?? '',
          cursor,
          next_cursor: page.next_cursor < count ? page.next_cursor : null,
        });
      } catch {
        return json({ error: 'Log unavailable' }, 503);
      }
    }
    if (url.pathname.startsWith('/admin/')) {
      if (url.origin !== 'https://internal.invalid' || url.search) return notFound();
      const json = (data, status = 200) =>
        Response.json(data, { status, headers: { 'Cache-Control': 'no-store' } });
      if (url.pathname === '/admin/list' && request.method === 'GET') {
        try {
          const entries = await this.ctx.storage.list({ prefix: 'record:', limit: 1000 });
          // Never label a truncated storage scan as the newest runs.
          if (entries.size >= 1000) return json({ error: 'Ledger limit reached' }, 503);
          const text = (v) => (typeof v === 'string' ? v.slice(0, 256) : null);
          const number = (v) => (Number.isFinite(v) ? v : null);
          const runs = [...entries.values()]
            .sort((a, b) => b.started - a.started || String(b.key).localeCompare(String(a.key)))
            .slice(0, 100)
            .map((r) => {
              const phase =
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
              return {
                run: text(r.key),
                sha: text(r.sha),
                branch: text(r.branch),
                profile: text(budgetClass(r)),
                tier: text(r.tier ?? null),
                family: text(r.family ?? null),
                batch: text(r.batch ?? null),
                die: text(r.die),
                gpu_model: text(r.hardware?.gpu_name ?? r.gpu),
                machine_id: providerId(r.machineId),
                host_id: providerId(r.hostId),
                instance_id: providerId(r.id),
                state: text(r.ended !== null ? 'NONE' : (r.closed ?? phase)),
                phase: text(phase),
                verdict: text(r.verdict),
                cost_estimate_usd: number(
                  ((r.computeStoragePrice ?? r.price) *
                    Math.max(0, (r.ended ?? Date.now()) - r.started)) /
                    3600000 +
                    (r.bandwidthCost ?? 0),
                ),
                started_at: number(r.started),
                ended_at: number(r.ended),
                boot_seconds: number((r.firstReportAt - r.started) / 1000),
              };
            });
          const budget = await new Ledger(this.ctx.storage).budgetSummary(
            configuration(this.env),
            Date.now(),
          );
          return json({ runs, budget, batches: await this.service.adminBatches() });
        } catch {
          return json({ error: 'List unavailable' }, 503);
        }
      }
      // Owner admin start and plan (2026-11-05, wtier). Reachable only through the admin Worker's
      // DO binding (internal origin), which checks the owner's Cloudflare Access identity. The
      // plan rents nothing; the start enters the same admission path as a PR-triggered rental.
      if (
        (url.pathname === '/admin/start' || url.pathname === '/admin/plan') &&
        request.method === 'POST'
      ) {
        let body;
        try {
          body = JSON.parse(
            new TextDecoder('utf-8', { fatal: true }).decode(await readBounded(request, 512)),
          );
        } catch {
          return json({ error: 'Invalid request' }, 400);
        }
        try {
          const answer =
            url.pathname === '/admin/plan'
              ? await this.service.adminPlan(body)
              : await this.service.adminStart(body);
          return json(answer);
        } catch (e) {
          return json(
            {
              error: e instanceof BrokerError ? e.state : 'UNAVAILABLE',
              ...(e instanceof BrokerError && e.reason ? { reason: e.reason } : {}),
              ...(e instanceof BrokerError && e.budget ? { budget: e.budget } : {}),
              ...(e instanceof BrokerError && e.plan ? { plan: e.plan } : {}),
            },
            e instanceof BrokerError ? e.status : 503,
          );
        }
      }
      if (url.pathname === '/admin/stop' && request.method === 'POST') {
        let selection;
        try {
          selection = JSON.parse(new TextDecoder().decode(await readBounded(request, 256)));
          if (
            !selection ||
            Object.keys(selection).length !== 1 ||
            !(
              selection.all === true ||
              (typeof selection.run === 'string' &&
                /^[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(selection.run)) ||
              (typeof selection.batch === 'string' && /^8[0-9]{18}$/.test(selection.batch))
            )
          )
            throw Error();
        } catch {
          return json({ error: 'Invalid selection' }, 400);
        }
        if (!validSecret(this.env.OWNER_STOP_TOKEN))
          return json({ error: 'Stop unavailable' }, 503);
        if (selection.batch !== undefined) {
          try {
            await this.service.adminStopBatch(selection.batch);
            return json({ state: 'NONE' });
          } catch {
            return json({ error: 'Stop unavailable' }, 503);
          }
        }
        // Enter exactly the existing owner stop path, including its queue and cleanup checks.
        const path = selection.all ? '/stop-all' : '/stop/' + selection.run;
        const prefix = this.env.BROKER_PATH_PREFIX ? '/' + this.env.BROKER_PATH_PREFIX : '';
        const response = await this.service.fetch(
          new Request(this.env.WORKER_URL + prefix + path, {
            method: 'POST',
            headers: { Authorization: 'Bearer ' + this.env.OWNER_STOP_TOKEN },
          }),
        );
        // The owner route's response contains logs/capability bindings; never relay those.
        return response.ok
          ? json({ state: 'NONE' })
          : json({ error: 'Stop unavailable' }, response.status);
      }
      return notFound();
    }
    if (
      new URL(request.url).origin === 'https://internal.invalid' &&
      new URL(request.url).pathname === '/cron'
    ) {
      await this.service.sweep();
      await this.service.storeTrees?.();
      return new Response('ok');
    }
    return this.service.fetch(request);
  }
}
// Buffer only the admitted prefix before entering the serialized ledger. An
// unfinished network upload must never hold its busy flag or block cron.
async function detachedUpload(request, path) {
  if (!request.body) return request;
  // Evidence: the declared length is checked before a byte is read; the buffer never exceeds the cap.
  if (/^\/evidence\//.test(path)) evidenceLength(request);
  const limit =
    (/^\/evidence\//.test(path)
      ? EVIDENCE_MAX_BYTES
      : /^\/report\//.test(path)
        ? 65536
        : /^\/kill\//.test(path)
          ? 256
          : path === '/rent'
            ? 2048
            : path === CHECK_PATH
              ? 1024
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
// A new identity requests initial placement; a hint cannot move the old ledger's state.
function placedLedger(env) {
  return env.LEDGER.get(env.LEDGER.idFromName('owner-shared-vast-account-enam-v1'), {
    locationHint: 'enam',
  });
}
export function createGateway({ authorize, fetcher = fetch, now = Date.now } = {}) {
  const verifier = new OidcVerifier(fetcher, now),
    local = new Map();
  let boxVerifier, boxSecret;
  let boxWindow = -1;
  const boxCounts = new Map();
  const evidenceCounts = new Map();
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
    if (/^\/(?:report|kill|blob-url|bench-data|evidence)\//.test(path)) {
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
        evidenceCounts.clear();
      }
      const used = boxCounts.get(key) ?? 0;
      if (used >= 300 || (!boxCounts.has(key) && boxCounts.size >= 1024))
        refuse('RATE_LIMITED', 429);
      boxCounts.set(key, used + 1);
      // An upload carries up to 8 MiB that the gateway buffers: at most 6 per minute per rental.
      if (path.startsWith('/evidence/')) {
        const sent = evidenceCounts.get(key) ?? 0;
        if (sent >= 6 || (!evidenceCounts.has(key) && evidenceCounts.size >= 1024))
          refuse('RATE_LIMITED', 429);
        evidenceCounts.set(key, sent + 1);
      }
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
        logRefusal(error, path);
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
      return placedLedger(env).fetch(buffered);
    },
    scheduled(_event, env, ctx) {
      ctx.waitUntil(
        placedLedger(env).fetch(new Request('https://internal.invalid/cron', { method: 'POST' })),
      );
    },
  };
}
export default createGateway();
