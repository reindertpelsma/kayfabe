// Independent, cron-only backstop. No broker imports, ledger, rental or public controls.
export function createReaper({
  fetcher = fetch,
  now = Date.now,
  report = (x) => console.log(JSON.stringify(x)),
} = {}) {
  async function sweep(env) {
    // Only local static stages reach the report; never inspect a caught provider error.
    let reason = 'configuration';
    const integer = (name, fallback, lo, hi) => {
      const value = env[name] ?? String(fallback);
      if (!/^[1-9][0-9]{0,3}$/.test(value) || Number(value) < lo || Number(value) > hi)
        throw Error('configuration');
      return Number(value);
    };
    try {
      const hard = integer('REAPER_HARD_MINUTES', 180, 1, 1440);
      const margin = integer('REAPER_MARGIN_MINUTES', 15, 15, 120);
      if (
        typeof env.REAPER_VAST_API_KEY !== 'string' ||
        !/^[\x21-\x7e]{1,1024}$/.test(env.REAPER_VAST_API_KEY)
      )
        throw Error('configuration');
      const request = async (path, method = 'GET') => {
        reason = 'transport';
        const response = await fetcher('https://console.vast.ai' + path, {
          method,
          redirect: 'manual',
          signal: AbortSignal.timeout(8000),
          headers: {
            Authorization: 'Bearer ' + env.REAPER_VAST_API_KEY,
            Accept: 'application/json',
          },
        });
        reason = 'http_refusal';
        if (response.status !== 200 || response.redirected) {
          await response.body?.cancel();
          throw Error('provider');
        }
        reason = 'invalid_response';
        const reader = response.body?.getReader();
        if (!reader) throw Error('provider');
        const chunks = [];
        let size = 0;
        try {
          while (true) {
            reason = 'transport';
            const { done, value } = await reader.read();
            if (done) break;
            size += value.length;
            if (size > 262144) {
              reason = 'invalid_response';
              throw Error('provider');
            }
            chunks.push(value);
          }
        } finally {
          await reader.cancel().catch(() => {});
        }
        const bytes = new Uint8Array(size);
        let offset = 0;
        for (const value of chunks) {
          bytes.set(value, offset);
          offset += value.length;
        }
        reason = 'invalid_json';
        return JSON.parse(
          new TextDecoder('utf-8', { fatal: true, ignoreBOM: false }).decode(bytes),
        );
      };
      const list = async () => {
        const data = await request('/api/v1/instances/?limit=100');
        reason = data == null ? 'malformed_list' : 'incomplete_list';
        if (
          data.success !== true ||
          !Array.isArray(data.instances) ||
          data.instances.length >= 100 ||
          data.next_token ||
          data.next
        )
          throw Error('incomplete list');
        reason = 'malformed_list';
        const ids = new Set();
        for (const row of data.instances) {
          if (!Number.isSafeInteger(row?.id) || row.id <= 0 || ids.has(row.id))
            throw Error('invalid list');
          ids.add(row.id);
        }
        return data.instances;
      };
      const overdue = (row) =>
        typeof row.label === 'string' &&
        /^kf-ci-[1-9][0-9]{0,19}-[1-9][0-9]{0,19}$/.test(row.label) &&
        typeof row.start_date === 'number' &&
        Number.isFinite(row.start_date) &&
        row.start_date >= 1000000000 &&
        row.start_date <= now() / 1000 &&
        now() / 1000 - row.start_date > (hard + margin) * 60;
      const first = (await list()).filter(overdue).slice(0, 20);
      if (!first.length) {
        report({ event: 'reaper_complete', deleted: 0 });
        return;
      }
      const second = new Map((await list()).map((row) => [row.id, row]));
      let deleted = 0,
        unconfirmed = 0,
        failed = 0;
      for (const row of first) {
        const current = second.get(row.id);
        if (
          !current ||
          !overdue(current) ||
          row.label !== current.label ||
          row.start_date !== current.start_date
        )
          continue;
        // One undeletable row must not shield the others; it is retried on the next tick.
        try {
          const ack = await request('/api/v0/instances/' + row.id + '/', 'DELETE');
          if (ack.success !== true) throw Error('provider');
        } catch {
          failed++;
          continue;
        }
        if ((await list()).some((item) => item.id === row.id)) unconfirmed++;
        else deleted++;
      }
      report({ event: 'reaper_complete', deleted, unconfirmed, failed });
    } catch {
      report({ event: 'reaper_unavailable', reason });
    }
  }
  return {
    fetch() {
      return new Response('Not found', { status: 404 });
    },
    sweep,
    scheduled(_event, env, ctx) {
      ctx.waitUntil(sweep(env));
    },
  };
}
export default createReaper();
