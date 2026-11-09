import { DEFAULTS } from './defaults.js';
import { faultPenalty, retryEligible } from './provider-policy.js';
// prettier-ignore
import {refuse,numericId,budgetRefuse} from './policy.js';
// prettier-ignore
import {logChain} from './tokens.js';
import { profileCaps, budgetClass } from './policy.js';
import { LOG_MAX_BYTES, workerPiece, boxPiece } from './publiclog.js';
// Queue of the public log and report of a settled leg (src/publish.js): plogq:<run key>:<leg>.
export const PUBLISH_PREFIX = 'plogq:';
export const publishEntryKey = (record) =>
  PUBLISH_PREFIX + record.key + ':' + (record.providerRetries ?? 0);
export const reportChunkKey = (key, leg, i) => 'rreport:' + key + ':' + leg + ':' + i;
// One leg's estimated provider spend; previous legs remain separate durable cost receipts.
export function rentalCost(record, at) {
  return (
    ((record.computeStoragePrice ?? record.price) * (at - record.started)) / 3600000 +
    (record.bandwidthCost ?? 0)
  );
}
export function costAttempt(record) {
  return { attemptKey: record.key, attemptStarted: record.originalStarted ?? record.started };
}
// Spend-row and record fields of a budget class (review F4): the stored `profile` is always one an
// older Worker knows (check or sweep); a release row adds `budgetClass: 'release'`.
export function spendClass(record) {
  const c = budgetClass(record);
  return c === 'release' ? { profile: 'check', budgetClass: 'release' } : { profile: c };
}
export class Ledger {
  constructor(storage) {
    this.storage = storage;
  }
  get(key) {
    return this.storage.get(key);
  }
  put(key, value) {
    return this.storage.put(key, value);
  }
  async markProviderFault(record, reason, now, policy = DEFAULTS.providerPolicy) {
    if (record.firstReportAt !== undefined) return;
    const identity = record.key + ':' + record.nonce;
    await this.storage.transaction(async (storage) => {
      for (const [prefix, id] of [
        ['provider-machine:', record.machineId],
        ['provider-host:', record.hostId],
      ]) {
        if (!Number.isSafeInteger(id) || id <= 0) continue;
        const key = prefix + id,
          rows = await storage.list({ prefix, limit: policy.historyIdentities + 1 });
        let history = rows.get(key) ?? { faults: [] };
        if (history.faults.some((f) => f.identity === identity)) continue;
        if (!rows.has(key) && rows.size >= policy.historyIdentities) {
          const oldest = [...rows].sort(
            (a, b) => (a[1].at ?? 0) - (b[1].at ?? 0) || a[0].localeCompare(b[0]),
          )[0];
          await storage.delete(oldest[0]);
        }
        const prior = history.faults.length,
          weight = 2 ** Math.min(prior, policy.historyPerIdentity);
        history = {
          at: now,
          faults: [...history.faults, { identity, at: now, reason, weight }].slice(
            -policy.historyPerIdentity,
          ),
        };
        await storage.put(key, history);
      }
    });
  }
  async migrateLegacyFaults(policy = DEFAULTS.providerPolicy) {
    for (const prefix of ['bad-machine:', 'bad-host:']) {
      const rows = await this.storage.list({ prefix, limit: 257 });
      if (rows.size > 256) continue;
      for (const [key, value] of rows) {
        if (
          value?.permanent === true ||
          !['PROVIDER_FAULT', 'KVM_DOMAIN_NOT_FOUND'].includes(value?.reason) ||
          !Number.isFinite(value?.at)
        )
          continue;
        const target = key.replace('bad-', 'provider-'),
          targetPrefix = target.startsWith('provider-machine:')
            ? 'provider-machine:'
            : 'provider-host:';
        const history = (await this.get(target)) ?? { at: value.at, faults: [] };
        const identity = 'legacy:' + key + ':' + value.at;
        if (!history.faults.some((f) => f.identity === identity)) {
          const histories = await this.storage.list({
            prefix: targetPrefix,
            limit: policy.historyIdentities + 1,
          });
          if (!histories.has(target) && histories.size >= policy.historyIdentities) {
            const oldest = [...histories].sort(
              (a, b) => (a[1].at ?? 0) - (b[1].at ?? 0) || a[0].localeCompare(b[0]),
            )[0];
            await this.storage.delete(oldest[0]);
          }
          history.faults = [
            ...history.faults,
            { identity, at: value.at, reason: value.reason, weight: 1 },
          ].slice(-policy.historyPerIdentity);
          await this.put(target, history);
        }
        await this.storage.delete(key);
      }
    }
  }
  async markProviderSuccess(record, policy = DEFAULTS.providerPolicy) {
    await this.storage.transaction(async (storage) => {
      for (const [prefix, id] of [
        ['provider-machine:', record.machineId],
        ['provider-host:', record.hostId],
      ]) {
        const key = prefix + id,
          history = await storage.get(key);
        if (!history || history.success === record.nonce) continue;
        history.faults = history.faults.map((f) => ({
          ...f,
          weight: f.weight * policy.successFactor,
        }));
        history.success = record.nonce;
        await storage.put(key, history);
      }
    });
  }
  async providerPenalty(machine, host, at, policy) {
    return Math.min(
      policy.penaltyCapUsdHour,
      faultPenalty(await this.get('provider-machine:' + machine), at, policy) +
        faultPenalty(await this.get('provider-host:' + host), at, policy),
    );
  }
  async save(record) {
    await this.put('record:' + record.key, record);
    if (record.id) await this.put('id:' + record.id, record.key);
  }
  async appendLog(record, text) {
    let chunk = '';
    const encoder = new TextEncoder();
    record.logChunks ??= 0;
    const flush = async () => {
      if (chunk) {
        record.logHash = await logChain(record.logHash ?? '0'.repeat(64), chunk);
        await this.put('log:' + record.key + ':' + record.logChunks, chunk);
        await this.put('log-hash:' + record.key + ':' + record.logChunks, record.logHash);
        record.logChunks++;
        chunk = '';
      }
    };
    for (const line of text.split(/(?<=\n)/)) {
      if (encoder.encode(chunk + line).length > 16384) await flush();
      chunk += line;
    }
    await flush();
  }
  // The text the box streamed, kept WHOLE for the public log (src/publiclog.js). Unlike appendLog
  // this is not display text: nothing is stripped here, the redaction happens when the log is
  // published. One storage value per post (a post carries at most 60000 bytes). The only limit is
  // the public log's own cap; what does not fit is counted (plogDropped) and said in the log.
  async appendPublicLog(record, text, cap = LOG_MAX_BYTES, worker = false) {
    if (typeof text !== 'string' || text === '') return;
    text = worker ? workerPiece(text) : boxPiece(text);
    const bytes = new TextEncoder().encode(text).length;
    record.plogChunks ??= 0;
    record.plogBytes ??= 0;
    if (record.plogBytes + bytes > cap) {
      record.plogDropped = (record.plogDropped ?? 0) + bytes;
      return;
    }
    await this.put('plog:' + record.key + ':' + record.plogChunks, text);
    record.plogChunks++;
    record.plogBytes += bytes;
  }
  async logPage(record, cursor, count = 4) {
    const end = Math.min(record.logChunks ?? 0, cursor + count);
    let text = '';
    const chunks = [];
    for (let i = cursor; i < end; i++) {
      const chunk = await this.get('log:' + record.key + ':' + i);
      if (typeof chunk !== 'string') refuse();
      text += chunk;
      chunks.push(chunk);
    }
    const hash = end ? await this.get('log-hash:' + record.key + ':' + (end - 1)) : '0'.repeat(64);
    if (typeof hash !== 'string') refuse();
    return {
      cursor,
      next_cursor: end,
      chunks,
      log_increment_sanitised: text,
      log_sha256: hash,
      bytes_dropped: record.fullBytesDropped ?? record.bytesDropped ?? 0,
    };
  }
  // A release (large-tier batch) holds the one-rental-per-commit gap for its legs: a leg is exempt
  // only while its release is RUNNING, tests the same commit and lists this key as PENDING. A
  // small or middle admin start is never exempt (no weaker than a PR-triggered rental).
  async batchLeg(id, key, sha) {
    if (typeof id !== 'string') return false;
    const batch = await this.get('batch:' + id);
    return (
      !!batch &&
      batch.id === id &&
      batch.state === 'RUNNING' &&
      batch.tier === 'large' &&
      batch.sha === sha &&
      batch.legs.some((l) => l.key === key && l.state === 'PENDING')
    );
  }
  async granted(run) {
    const key = await this.get('run:' + run);
    return key ? this.get('record:' + key) : null;
  }
  async grant(run, record) {
    await this.put('run:' + run, record.key);
  }
  async openRecords() {
    const entries = await this.storage.list({ prefix: 'open:', limit: 1000 });
    if (entries.size >= 1000) refuse();
    const records = [];
    for (const [key] of entries) {
      const record = await this.get('record:' + key.slice(5));
      if (!record) {
        console.log(JSON.stringify({ event: 'incomplete_ledger_index' }));
        continue;
      }
      if (record.ended !== null) {
        await this.storage.delete(key);
        continue;
      }
      records.push(record);
    }
    return records;
  }
  // All ledger admission checks and the intent writes share ONE transaction. The
  // service's preflight reads/queue are only early refusals, never this lock.
  // No provider operation belongs here: storage may retry the callback.
  async reserve(record, { cfg, now, cost, credit, retryNonce }) {
    await this.storage.transaction(async (storage) => {
      const ledger = new Ledger(storage);
      await ledger.migrateLegacyFaults(cfg.provider);
      const previous = await ledger.get('cost:' + record.key);
      if (retryNonce !== undefined) {
        const prior = await ledger.granted(record.key),
          spent = prior ? (prior.retryCostUsd ?? 0) + rentalCost(prior, prior.ended) : NaN;
        if (
          !previous ||
          !prior ||
          prior.nonce !== retryNonce ||
          prior.verdict !== 'PROVIDER_FAULT' ||
          prior.closed !== 'PROVIDER_FAULT' ||
          !Number.isFinite(prior.ended) ||
          (!prior.id && !prior.noInstance) ||
          !retryEligible(prior, now, cfg.provider) ||
          record.providerRetries !== (prior.providerRetries ?? 0) + 1 ||
          record.nonce === prior.nonce ||
          record.machineId === prior.machineId ||
          !Number.isSafeInteger(prior.hostId) ||
          prior.hostId <= 0 ||
          !Number.isSafeInteger(record.hostId) ||
          record.hostId <= 0 ||
          record.hostId === prior.hostId ||
          (prior.rentalAttempts ?? []).some(
            (a) => a.machine === record.machineId || a.host === record.hostId,
          ) ||
          JSON.stringify(record.rentalAttempts ?? []) !==
            JSON.stringify(prior.rentalAttempts ?? []) ||
          !['run', 'attempt', 'sha', 'branch', 'profile', 'runnerNonce'].every(
            (key) => record[key] === prior[key],
          ) ||
          record.hardDeadline !== prior.hardDeadline ||
          record.softDeadline !== prior.softDeadline ||
          record.originalStarted !== (prior.originalStarted ?? prior.started) ||
          record.sessionDeadline !== prior.sessionDeadline ||
          record.started < prior.ended ||
          record.runBudgetUsd !== prior.runBudgetUsd ||
          !Number.isFinite(spent) ||
          spent < 0 ||
          record.retryCostUsd !== spent ||
          !Number.isFinite(prior.runBudgetUsd) ||
          spent + cost > prior.runBudgetUsd ||
          prior.hardDeadline <= now + cfg.softMargin ||
          previous.ended !== prior.ended ||
          (await ledger.get('cost:' + record.key + ':provider:' + (prior.providerRetries ?? 0)))
        )
          refuse('NOT_ELIGIBLE', 409);
        // Preserve real spend/count before replacing this run's current receipt. Every proof,
        // cap check, archive and intent is one transaction; a racing replacement loses here.
        await ledger.put('cost:' + record.key + ':provider:' + (prior.providerRetries ?? 0), {
          ...previous,
          ...costAttempt(prior),
          created: previous.created,
        });
        await storage.delete('cost:' + record.key);
        await storage.delete('ended:' + record.key);
        if ((await ledger.get('id:' + prior.id)) === record.key)
          await storage.delete('id:' + prior.id);
      } else if (previous)
        budgetRefuse('cost-key', { started_at: previous.started, ended_at: previous.ended });
      const cached = await ledger.get('sha:' + record.sha),
        force = Number(record.attempt ?? record.key.split('-').at(-1)) > 1,
        leg =
          record.batch !== undefined &&
          (await ledger.batchLeg(record.batch, record.key, record.sha));
      if (retryNonce === undefined && !leg && cached && (!force || now - cached.started < cfg.gap))
        budgetRefuse('sha-gap', { gap_ms: cfg.gap, elapsed_ms: Math.max(0, now - cached.started) });
      const open = await ledger.openRecords(),
        branch = open.filter((o) => o.branch === record.branch && o.sha !== record.sha);
      if (branch.some((o) => numericId(o.run) && BigInt(o.run) > BigInt(record.run)))
        refuse('NOT_ELIGIBLE', 409);
      // Let the next preflight supersede an older branch run, outside the txn.
      if (branch.length) refuse('BUSY', 409);
      if (
        open.length >= cfg.maxConcurrent ||
        open.filter((o) => budgetClass(o) === budgetClass(record)).length >=
          cfg.maxConcurrentProfile[budgetClass(record)]
      )
        refuse('BUSY', 409);
      const bad = await storage.list({ prefix: 'bad-machine:', limit: 257 });
      const badHosts = await storage.list({ prefix: 'bad-host:', limit: 257 });
      if (
        bad.size > 256 ||
        badHosts.size > 256 ||
        badHosts.has('bad-host:' + record.hostId) ||
        cfg.badHosts.includes(record.hostId) ||
        bad.has('bad-machine:' + record.machineId) ||
        cfg.badMachines.includes(record.machineId) ||
        open.some((o) => Number.isSafeInteger(o.machineId) && o.machineId === record.machineId)
      )
        refuse();
      await ledger.budgets(cfg, now, cost, null, {
        credit,
        sameAttempt: retryNonce !== undefined,
        attemptKey: record.key,
        attemptStarted: record.originalStarted ?? record.started,
      });
      await ledger.#writeReservation(record);
    });
  }
  async #writeReservation(record) {
    await this.save(record);
    // Owner ruling 2026-10-24 item 8: the open: index is the set of active runs.
    await this.put('open:' + record.key, true);
    await this.grant(record.key, record);
    // Review F6: a release leg never writes the one-rental-per-commit marker, so a release of a
    // commit never refuses that commit's own master gate with BUDGET:sha-gap.
    if (budgetClass(record) !== 'release')
      await this.put('sha:' + record.sha, { key: record.key, started: record.started });
    await this.put('cost:' + record.key, {
      ...costAttempt(record),
      ...spendClass(record),
      started: record.started,
      ended: null,
      created: record.id ? true : null,
      price: record.computeStoragePrice ?? record.price,
      bandwidthCost: record.bandwidthCost ?? 0,
      ttl: record.billingTtl ?? record.ttl,
    });
  }
  async settle(record, now) {
    record.ended = now;
    // End evidence, billing and the open index must commit together, including under faults.
    await this.storage.transaction(async (storage) => {
      const ledger = new Ledger(storage);
      await ledger.save(record);
      const created = record.id
        ? true
        : record.noInstance || record.closed === 'GONE'
          ? false
          : null;
      await ledger.put('cost:' + record.key, {
        ...costAttempt(record),
        ...spendClass(record),
        started: record.started,
        ended: now,
        created,
        price: created === false ? 0 : (record.computeStoragePrice ?? record.price),
        bandwidthCost: created === false ? 0 : (record.bandwidthCost ?? 0),
        ttl: record.billingTtl ?? record.ttl,
      });
      await storage.delete('open:' + record.key);
      await ledger.put('ended:' + record.key, now);
      // The public log and the run report of this leg are published by the cron, off this queue
      // (src/publish.js). The entry holds only what that job needs, fixed at this moment: the
      // chunk counts and the values to redact (a replacement leg changes the record).
      if ((record.plogChunks ?? 0) > 0 || record.runReport) {
        await ledger.put(publishEntryKey(record), {
          attempts: 0,
          chunks: record.plogChunks ?? 0,
          dropped: record.plogDropped ?? 0,
          rchunks: record.runReport?.chunks ?? 0,
          leg: record.providerRetries ?? 0,
          key: record.key,
          run: record.run,
          attempt: record.attempt,
          sha: record.sha,
          nonce: record.nonce,
          // The values the box held in earlier legs of this run or at rent time (review of #282, F4/F5).
          priorNonces: record.priorNonces ?? [],
          known: record.known ?? [],
          runnerNonce: record.runnerNonce ?? null,
          reportExpiry: record.reportExpiry,
          at: now,
        });
      }
      if ((await ledger.get('active')) === record.key) await storage.delete('active');
    });
  }
  // Owner ruling 2026-10-24: the log relay is not persistent. LOG_RETENTION_HOURS after
  // ended_at the cron deletes a run's log chunks, log-hash entries and record (at most four
  // runs per tick). Spend rows stay for the caps (pruned after 30 days by budgets()); sha-gap
  // rows stay for max(gap, 30 days); daily blob counters for two days.
  async expire(now, cfg) {
    let removed = 0;
    for (const [index, at] of await this.storage.list({ prefix: 'ended:', limit: 1000 })) {
      if (removed >= 4) break;
      if (typeof at === 'number' && now < at + cfg.logRetention) continue;
      const key = index.slice('ended:'.length),
        record = await this.get('record:' + key);
      // Preserve confirmed legacy no-instance proof before deleting its only old record.
      if (
        record &&
        record.id == null &&
        record.closed === 'GONE' &&
        Number.isFinite(record.ended)
      ) {
        const cost = await this.get('cost:' + key);
        if (cost && cost.created === undefined && cost.started === record.started)
          await this.put('cost:' + key, {
            ...cost,
            ended: record.ended,
            created: false,
            price: 0,
            bandwidthCost: 0,
          });
      }
      for (let i = 0; i < (record?.logChunks ?? 0); i++) {
        await this.storage.delete('log:' + key + ':' + i);
        await this.storage.delete('log-hash:' + key + ':' + i);
      }
      // The public log's pieces, the report chunks of every leg and the publish entries of this run.
      for (let i = 0; i < (record?.plogChunks ?? 0); i++)
        await this.storage.delete('plog:' + key + ':' + i);
      for (const prefix of ['rreport:' + key + ':', PUBLISH_PREFIX + key + ':'])
        for (const [name] of await this.storage.list({ prefix, limit: 256 }))
          await this.storage.delete(name);
      if (record?.id && (await this.get('id:' + record.id)) === key)
        await this.storage.delete('id:' + record.id);
      for (const name of ['run:', 'run-pr:', 'record:']) await this.storage.delete(name + key);
      await this.storage.delete(index);
      removed++;
    }
    const horizon = now - Math.max(cfg.gap, 30 * 86400000);
    for (const [key, value] of await this.storage.list({ prefix: 'sha:', limit: 1000 }))
      if (Number.isFinite(value?.started) && value.started < horizon)
        await this.storage.delete(key);
    const today = Math.floor(now / 86400000);
    for (const [key] of await this.storage.list({ prefix: 'blob-day:', limit: 1000 }))
      if (Number(key.slice('blob-day:'.length)) < today - 1) await this.storage.delete(key);
  }
  // Read-only by default, including the owner/admin path. Admission may prune expired rows.
  // Daily caps/counts are per profile; rolling spend and remaining credit liability are shared.
  async budgetSummary(cfg, now, excludeKey = null, prune = false, attemptKeyForCount = null) {
    const entries = await this.storage.list({ prefix: 'cost:', limit: 1000 });
    if (entries.size >= 1000) budgetRefuse('ledger-full', { rows: entries.size, rows_cap: 1000 });
    const day = Math.floor(now / 86400000) * 86400000,
      rolling = now - 30 * 86400000,
      profiles = {
        check: { daily_usd: 0, count_today: 0 },
        sweep: { daily_usd: 0, count_today: 0 },
        release: { daily_usd: 0, count_today: 0 },
      },
      rows = [],
      counted = { check: new Set(), sweep: new Set(), release: new Set() };
    let monthly = 0,
      outstanding = 0;
    for (const [key, stored] of entries) {
      let v = stored;
      if (key === 'cost:' + excludeKey) continue;
      // A settled legacy failure can retain an old priced cost receipt. Durable no-instance
      // evidence applies even when that receipt already has an end (and no created marker).
      if (v.created === undefined) {
        const record = await this.get('record:' + key.slice(5));
        if (
          record &&
          record.id == null &&
          record.closed === 'GONE' &&
          Number.isFinite(record.ended) &&
          record.started === v.started
        ) {
          v = { ...v, ended: record.ended, created: false, price: 0, bandwidthCost: 0 };
          if (prune) await this.put(key, v);
        }
      }
      // Recover old partial settlements from the authoritative durable record, without writes.
      if (v.ended === null) {
        const record = await this.get('record:' + key.slice(5));
        if (Number.isFinite(record?.ended) && record.started === v.started) {
          const created = record.id
            ? true
            : record.noInstance || record.closed === 'GONE'
              ? false
              : null;
          v = {
            ...v,
            ended: record.ended,
            created,
            price:
              created === false ? 0 : Math.max(v.price, record.computeStoragePrice ?? record.price),
            bandwidthCost:
              created === false ? 0 : Math.max(v.bandwidthCost ?? 0, record.bandwidthCost ?? 0),
          };
        }
      }
      if (
        !Number.isFinite(v.started) ||
        !Number.isFinite(v.price) ||
        v.price < 0 ||
        v.started > now ||
        (v.ended !== null && !Number.isFinite(v.ended)) ||
        (v.ended === null && (!Number.isFinite(v.ttl) || v.ttl < 0)) ||
        !Number.isFinite(v.bandwidthCost ?? 0) ||
        (v.bandwidthCost ?? 0) < 0
      )
        refuse();
      const attemptKey = v.attemptKey ?? key.slice(5),
        attemptStarted = v.attemptStarted ?? v.started;
      if (
        (v.attemptKey !== undefined || v.attemptStarted !== undefined) &&
        (typeof v.attemptKey !== 'string' ||
          !attemptKey ||
          attemptKey.length > 128 ||
          !(
            key === 'cost:' + attemptKey ||
            (key.startsWith('cost:' + attemptKey + ':provider:') &&
              /^(?:[0-9]|[12][0-9]|3[01])$/.test(
                key.slice(('cost:' + attemptKey + ':provider:').length),
              ))
          ) ||
          !Number.isFinite(v.attemptStarted) ||
          attemptStarted < 0 ||
          attemptStarted > v.started)
      )
        refuse();
      const end = v.ended ?? now;
      if (end < v.started) refuse();
      const bandwidth = v.bandwidthCost ?? 0,
        remaining =
          v.ended === null ? (Math.max(0, v.started + v.ttl - now) * v.price) / 3600000 : 0,
        profile = budgetClass(v);
      if (!Object.hasOwn(profiles, profile)) refuse();
      // New receipts distinguish confirmed no-instance failures from actual/free rentals.
      // Legacy zero rows need the still-retained durable record as evidence; unknowns count.
      let uncreated = v.ended !== null && v.created === false;
      if (v.created === undefined && v.ended !== null && v.price === 0 && bandwidth === 0) {
        const record = await this.get('record:' + key.slice(5));
        uncreated =
          record != null &&
          record.id == null &&
          record.closed === 'GONE' &&
          Number.isFinite(record.ended);
      }
      if (uncreated && (v.price !== 0 || bandwidth !== 0)) refuse();
      const daily =
          (Math.max(0, end - Math.max(v.started, day)) * v.price) / 3600000 +
          remaining +
          (end >= day ? bandwidth : 0),
        billed = (Math.max(0, end - v.started) * v.price) / 3600000 + bandwidth,
        reserved = v.ended === null ? remaining + bandwidth : 0;
      profiles[profile].daily_usd += daily;
      monthly +=
        (Math.max(0, end - Math.max(v.started, rolling)) * v.price) / 3600000 +
        remaining +
        (end >= rolling ? bandwidth : 0);
      if (attemptStarted >= day && !uncreated && !counted[profile].has(attemptKey)) {
        counted[profile].add(attemptKey);
        profiles[profile].count_today++;
      }
      outstanding += reserved;
      rows.push({
        key: key.slice(5),
        profile,
        started: v.started,
        ended: v.ended,
        price: v.price,
        billed_usd: billed,
        reserved_usd: reserved,
      });
      if (prune && v.ended !== null && end < rolling) await this.storage.delete(key);
    }
    rows.sort((a, b) => b.started - a.started || b.key.localeCompare(a.key));
    return {
      daily_usd: profiles.check.daily_usd + profiles.sweep.daily_usd + profiles.release.daily_usd,
      count_today:
        profiles.check.count_today + profiles.sweep.count_today + profiles.release.count_today,
      profiles,
      ...(attemptKeyForCount === null
        ? {}
        : { counted_attempt: counted[cfg.profile ?? 'check'].has(attemptKeyForCount) }),
      rolling_usd: monthly,
      outstanding_usd: outstanding,
      outstanding_rows: rows.filter((v) => v.ended === null),
      spend_rows: rows.slice(0, 20),
    };
  }
  // Returns only remaining compute plus transfer, not elapsed spend, for the credit reserve.
  async budgets(cfg, now, cost, excludeKey = null, options = {}) {
    if (!Number.isFinite(cost) || cost < 0) budgetRefuse('invalid-cost', { cost_usd: cost });
    const summary = await this.budgetSummary(
        cfg,
        now,
        excludeKey,
        true,
        options.attemptKey ?? null,
      ),
      profile = cfg.profile ?? 'check',
      caps = profileCaps(cfg, profile),
      values = {
        ...summary.profiles[profile],
        rolling_usd: summary.rolling_usd,
        outstanding_usd: summary.outstanding_usd,
        cost_usd: cost,
        daily_cap_usd: caps.dailyUsd,
        count_cap: caps.dailyCount,
        rolling_cap_usd: cfg.rolling,
        reserve_usd: cfg.reserve,
        ...(Object.hasOwn(options, 'credit')
          ? {
              credit_usd: options.credit,
              credit_required_usd: cfg.reserve + cost + summary.outstanding_usd,
            }
          : {}),
      };
    if (Object.hasOwn(options, 'credit') && !Number.isFinite(options.credit))
      budgetRefuse('no-credit', values);
    const sameAttempt =
      options.sameAttempt &&
      (options.attemptKey === undefined ||
        summary.counted_attempt ||
        options.attemptStarted < Math.floor(now / 86400000) * 86400000);
    if (!sameAttempt && values.count_today >= caps.dailyCount) budgetRefuse('daily-count', values);
    if (values.daily_usd + cost > values.daily_cap_usd) budgetRefuse('daily-usd', values);
    // Review F8: the account-wide daily cap across profiles (same closed reason vocabulary).
    if (summary.daily_usd + cost > cfg.accountDaily)
      budgetRefuse('daily-usd', {
        ...values,
        daily_usd: summary.daily_usd,
        daily_cap_usd: cfg.accountDaily,
      });
    if (values.rolling_usd + cost > cfg.rolling) budgetRefuse('rolling-usd', values);
    if (Object.hasOwn(options, 'credit') && options.credit < values.credit_required_usd)
      budgetRefuse('credit-reserve', values);
    return summary.outstanding_usd;
  }
}
