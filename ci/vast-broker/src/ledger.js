// prettier-ignore
import {refuse} from './policy.js';
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
  async logPage(record, cursor) {
    const end = Math.min(record.logChunks ?? 0, cursor + 4);
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
  async active() {
    const key = await this.get('active');
    return key ? this.get('record:' + key) : null;
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
  async reserve(record) {
    await this.save(record);
    await this.put('open:' + record.key, true);
    await this.put('active', record.key);
    await this.grant(record.key, record);
    await this.put('sha:' + record.sha, { key: record.key, started: record.started });
    await this.put('cost:' + record.key, {
      profile: record.profile ?? 'check',
      started: record.started,
      ended: null,
      price: record.computeStoragePrice ?? record.price,
      bandwidthCost: record.bandwidthCost ?? 0,
      ttl: record.billingTtl ?? record.ttl,
    });
  }
  async settle(record, now) {
    record.ended = now;
    await this.save(record);
    await this.put('cost:' + record.key, {
      profile: record.profile ?? 'check',
      started: record.started,
      ended: now,
      price: record.computeStoragePrice ?? record.price,
      bandwidthCost: record.bandwidthCost ?? 0,
      ttl: record.billingTtl ?? record.ttl,
    });
    await this.storage.delete('open:' + record.key);
    await this.put('ended:' + record.key, now);
    if ((await this.get('active')) === record.key) await this.put('active', null);
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
  async budgets(cfg, now, cost, excludeKey = null) {
    const entries = await this.storage.list({ prefix: 'cost:', limit: 1000 });
    if (entries.size >= 1000) refuse('UNAVAILABLE_BUDGET', 409);
    const day = Math.floor(now / 86400000) * 86400000,
      rolling = now - 30 * 86400000;
    let daily = 0,
      monthly = 0,
      count = 0;
    for (const [key, v] of entries) {
      if (key === 'cost:' + excludeKey) continue;
      if (
        !Number.isFinite(v.started) ||
        !Number.isFinite(v.price) ||
        v.price < 0 ||
        v.started > now ||
        (v.ended !== null && !Number.isFinite(v.ended))
      )
        refuse();
      if (
        typeof (v.bandwidthCost ?? 0) !== 'number' ||
        !Number.isFinite(v.bandwidthCost ?? 0) ||
        (v.bandwidthCost ?? 0) < 0
      )
        refuse();
      const end = v.ended ?? now;
      if (end < v.started) refuse();
      const remaining =
        v.ended === null ? (Math.max(0, v.started + v.ttl - now) * v.price) / 3600000 : 0;
      if ((v.profile ?? 'check') === (cfg.profile ?? 'check'))
        daily +=
          (Math.max(0, end - Math.max(v.started, day)) * v.price) / 3600000 +
          remaining +
          (end >= day ? (v.bandwidthCost ?? 0) : 0);
      monthly +=
        (Math.max(0, end - Math.max(v.started, rolling)) * v.price) / 3600000 +
        remaining +
        (end >= rolling ? (v.bandwidthCost ?? 0) : 0);
      if (v.started >= day && (v.profile ?? 'check') === (cfg.profile ?? 'check')) count++;
      if (v.ended !== null && end < rolling) await this.storage.delete(key);
    }
    if (count >= cfg.dailyCount) refuse('UNAVAILABLE_BUDGET', 409);
    if (daily + cost > (cfg.profile === 'sweep' ? cfg.sweepDaily : cfg.daily))
      refuse('UNAVAILABLE_BUDGET', 409);
    if (monthly + cost > cfg.rolling) refuse('UNAVAILABLE_BUDGET', 409);
  }
}
