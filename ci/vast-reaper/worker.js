// Independent, cron-only backstop. No broker imports, ledger, rental or public controls.
export function createReaper({
  fetcher = fetch,
  now = Date.now,
  report = (x) => console.log(JSON.stringify(x)),
} = {}) {
  async function sweep(env) {
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
        const response = await fetcher('https://console.vast.ai' + path, {
          method,
          redirect: 'error',
          signal: AbortSignal.timeout(8000),
          headers: {
            Authorization: 'Bearer ' + env.REAPER_VAST_API_KEY,
            Accept: 'application/json',
          },
        });
        if (response.status !== 200 || response.redirected) {
          await response.body?.cancel();
          throw Error('provider');
        }
        const reader = response.body?.getReader();
        if (!reader) throw Error('provider');
        const chunks = [];
        let size = 0;
        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            size += value.length;
            if (size > 262144) throw Error('provider');
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
        return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
      };
      const list = async () => {
        const data = await request('/api/v1/instances/?limit=100');
        if (
          data.success !== true ||
          !Array.isArray(data.instances) ||
          data.instances.length >= 100 ||
          data.next_token ||
          data.next
        )
          throw Error('incomplete list');
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
        unconfirmed = 0;
      for (const row of first) {
        const current = second.get(row.id);
        if (
          !current ||
          !overdue(current) ||
          row.label !== current.label ||
          row.start_date !== current.start_date
        )
          continue;
        const ack = await request('/api/v0/instances/' + row.id + '/', 'DELETE');
        if (ack.success !== true) throw Error('provider');
        if ((await list()).some((item) => item.id === row.id)) unconfirmed++;
        else deleted++;
      }
      report({ event: 'reaper_complete', deleted, unconfirmed });
    } catch {
      report({ event: 'reaper_unavailable' });
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
