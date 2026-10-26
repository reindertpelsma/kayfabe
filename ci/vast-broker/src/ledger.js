// prettier-ignore
import {refuse,numericId,budgetRefuse} from './policy.js';
// prettier-ignore
import {logChain} from './tokens.js';
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
  async reserve(record, { cfg, now, cost, credit }) {
    await this.storage.transaction(async (storage) => {
      const ledger = new Ledger(storage);
      const previous = await ledger.get('cost:' + record.key);
      if (previous)
        budgetRefuse('cost-key', { started_at: previous.started, ended_at: previous.ended });
      const cached = await ledger.get('sha:' + record.sha),
        force = Number(record.attempt ?? record.key.split('-').at(-1)) > 1;
      if (cached && (!force || now - cached.started < cfg.gap))
        budgetRefuse('sha-gap', { gap_ms: cfg.gap, elapsed_ms: Math.max(0, now - cached.started) });
      const open = await ledger.openRecords(),
        branch = open.filter((o) => o.branch === record.branch && o.sha !== record.sha);
      if (branch.some((o) => numericId(o.run) && BigInt(o.run) > BigInt(record.run)))
        refuse('NOT_ELIGIBLE', 409);
      // Let the next preflight supersede an older branch run, outside the txn.
      if (branch.length) refuse('BUSY', 409);
      if (
        open.length >= cfg.maxConcurrent ||
        open.filter((o) => (o.profile ?? 'check') === (record.profile ?? 'check')).length >=
          cfg.maxConcurrentProfile[record.profile ?? 'check']
      )
        refuse('BUSY', 409);
      const bad = await storage.list({ prefix: 'bad-machine:', limit: 257 });
      if (
        bad.size > 256 ||
        bad.has('bad-machine:' + record.machineId) ||
        cfg.badMachines.includes(record.machineId) ||
        open.some((o) => Number.isSafeInteger(o.machineId) && o.machineId === record.machineId)
      )
        refuse();
      await ledger.budgets(cfg, now, cost, null, { credit });
      await ledger.#writeReservation(record);
    });
  }
  async #writeReservation(record) {
    await this.save(record);
    // Owner ruling 2026-10-24 item 8: the open: index is the set of active runs.
    await this.put('open:' + record.key, true);
    await this.grant(record.key, record);
    await this.put('sha:' + record.sha, { key: record.key, started: record.started });
    await this.put('cost:' + record.key, {
      profile: record.profile ?? 'check',
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
      const created = record.id ? true : record.closed === 'GONE' ? false : null;
      await ledger.put('cost:' + record.key, {
        profile: record.profile ?? 'check',
        started: record.started,
        ended: now,
        created,
        price: created === false ? 0 : (record.computeStoragePrice ?? record.price),
        bandwidthCost: created === false ? 0 : (record.bandwidthCost ?? 0),
        ttl: record.billingTtl ?? record.ttl,
      });
      await storage.delete('open:' + record.key);
      await ledger.put('ended:' + record.key, now);
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
      for (let i = 0; i < (record?.logChunks ?? 0); i++) {
        await this.storage.delete('log:' + key + ':' + i);
        await this.storage.delete('log-hash:' + key + ':' + i);
      }
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
  async budgetSummary(cfg, now, excludeKey = null, prune = false) {
    const entries = await this.storage.list({ prefix: 'cost:', limit: 1000 });
    if (entries.size >= 1000) budgetRefuse('ledger-full', { rows: entries.size, rows_cap: 1000 });
    const day = Math.floor(now / 86400000) * 86400000,
      rolling = now - 30 * 86400000,
      profiles = {
        check: { daily_usd: 0, count_today: 0 },
        sweep: { daily_usd: 0, count_today: 0 },
      },
      rows = [];
    let monthly = 0,
      outstanding = 0;
    for (const [key, stored] of entries) {
      let v = stored;
      if (key === 'cost:' + excludeKey) continue;
      // Recover old partial settlements from the authoritative durable record, without writes.
      if (v.ended === null) {
        const record = await this.get('record:' + key.slice(5));
        if (Number.isFinite(record?.ended) && record.started === v.started) {
          const created = record.id ? true : record.closed === 'GONE' ? false : null;
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
      const end = v.ended ?? now;
      if (end < v.started) refuse();
      const bandwidth = v.bandwidthCost ?? 0,
        remaining =
          v.ended === null ? (Math.max(0, v.started + v.ttl - now) * v.price) / 3600000 : 0,
        profile = v.profile ?? 'check';
      if (!Object.hasOwn(profiles, profile)) refuse();
      // New receipts distinguish confirmed no-instance failures from actual/free rentals.
      // Legacy zero rows need the still-retained durable record as evidence; unknowns count.
      let uncreated = v.ended !== null && v.created === false;
      if (v.created === undefined && v.ended !== null && v.price === 0 && bandwidth === 0) {
        const record = await this.get('record:' + key.slice(5));
        uncreated =
          record?.id === null && record.closed === 'GONE' && Number.isFinite(record.ended);
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
      if (v.started >= day && !uncreated) profiles[profile].count_today++;
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
      daily_usd: profiles.check.daily_usd + profiles.sweep.daily_usd,
      count_today: profiles.check.count_today + profiles.sweep.count_today,
      profiles,
      rolling_usd: monthly,
      outstanding_usd: outstanding,
      outstanding_rows: rows.filter((v) => v.ended === null),
      spend_rows: rows.slice(0, 20),
    };
  }
  // Returns only remaining compute plus transfer, not elapsed spend, for the credit reserve.
  async budgets(cfg, now, cost, excludeKey = null, options = {}) {
    if (!Number.isFinite(cost) || cost < 0) budgetRefuse('invalid-cost', { cost_usd: cost });
    const summary = await this.budgetSummary(cfg, now, excludeKey, true),
      profile = cfg.profile ?? 'check',
      values = {
        ...summary.profiles[profile],
        rolling_usd: summary.rolling_usd,
        outstanding_usd: summary.outstanding_usd,
        cost_usd: cost,
        daily_cap_usd: profile === 'sweep' ? cfg.sweepDaily : cfg.daily,
        count_cap: cfg.dailyCount,
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
    if (values.count_today >= cfg.dailyCount) budgetRefuse('daily-count', values);
    if (values.daily_usd + cost > values.daily_cap_usd) budgetRefuse('daily-usd', values);
    if (values.rolling_usd + cost > cfg.rolling) budgetRefuse('rolling-usd', values);
    if (Object.hasOwn(options, 'credit') && options.credit < values.credit_required_usd)
      budgetRefuse('credit-reserve', values);
    return summary.outstanding_usd;
  }
}
