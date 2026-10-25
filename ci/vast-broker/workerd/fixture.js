// Test-only entrypoint. No test route is exported by the production Worker.
import worker, { BrokerLedger, createService } from '../src/index.js';
import { Ledger } from '../src/ledger.js';
import { configuration } from '../src/policy.js';

export default worker;
export { BrokerLedger };

// A real workerd DO with real SQLite-backed storage. Independent service closures
// deliberately remove the in-memory queue from the admission safety argument.
export class AdmissionFixture {
  constructor(ctx, env) {
    this.ctx = ctx;
    this.env = env;
  }
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === '/seed') {
      for (const [key, value] of await request.json()) await this.ctx.storage.put(key, value);
      return new Response('ok');
    }
    if (path === '/snapshot') {
      return Response.json([...(await this.ctx.storage.list())]);
    }
    if (path === '/reserve') {
      const { record, config, cost = 0.1, credit = 20 } = await request.json();
      try {
        await new Ledger(this.ctx.storage).reserve(record, {
          cfg: { ...configuration(this.env), ...config },
          now: record.started,
          cost,
          credit,
        });
        return Response.json({ state: 'RESERVED' });
      } catch (error) {
        return Response.json({ state: error.state ?? 'ERROR' }, { status: error.status ?? 500 });
      }
    }
    const run = request.headers.get('X-Test-Run');
    const fetcher = (url, init = {}) => {
      const headers = new Headers(init.headers);
      headers.set('X-Test-Run', run);
      return fetch(url, { ...init, headers });
    };
    return createService({ env: this.env, storage: this.ctx.storage, fetch: fetcher }).fetch(
      request,
    );
  }
}
