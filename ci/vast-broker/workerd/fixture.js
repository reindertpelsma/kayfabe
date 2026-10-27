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
      return Response.json({ state: 'SEEDED' });
    }
    if (path === '/admin/list') return new BrokerLedger(this.ctx, this.env).fetch(request);
    if (path === '/settle') {
      const { record, now, failAt } = await request.json();
      const failing = (storage) => ({
        get: (...args) => storage.get(...args),
        list: (...args) => storage.list(...args),
        delete: (...args) => storage.delete(...args),
        put: async (key, value) => {
          await storage.put(key, value);
          if (failAt && key.startsWith(failAt)) throw Error('injected settlement fault');
        },
        transaction: (action) => storage.transaction((txn) => action(failing(txn))),
      });
      try {
        await new Ledger(failing(this.ctx.storage)).settle(record, now);
        return Response.json({ state: 'SETTLED' });
      } catch {
        return Response.json({ state: 'ERROR' }, { status: 503 });
      }
    }
    if (path === '/expire') {
      const { now } = await request.json();
      await new Ledger(this.ctx.storage).expire(now, {
        ...configuration(this.env),
        logRetention: 1,
      });
      return Response.json({ state: 'EXPIRED' });
    }
    if (path === '/snapshot') {
      return Response.json([...(await this.ctx.storage.list())]);
    }
    if (path === '/provider-fault') {
      const { record, reason = 'PROVIDER_FAULT', now = Date.now(), failAt } = await request.json();
      const failing = (storage) => ({
        list: (...args) => storage.list(...args),
        put: async (key, value) => {
          await storage.put(key, value);
          if (key.startsWith(failAt)) throw Error('injected provider fault write');
        },
        transaction: (action) => storage.transaction((txn) => action(failing(txn))),
      });
      try {
        await new Ledger(failAt ? failing(this.ctx.storage) : this.ctx.storage).markProviderFault(
          record,
          reason,
          now,
        );
        return Response.json({ state: 'RECORDED' });
      } catch {
        return Response.json({ state: 'ERROR' }, { status: 503 });
      }
    }
    if (path === '/reserve') {
      const { record, config, cost = 0.1, credit = 20, failAt, retryNonce } = await request.json();
      const failing = (storage) => ({
        get: (...args) => storage.get(...args),
        list: (...args) => storage.list(...args),
        delete: (...args) => storage.delete(...args),
        put: async (key, value) => {
          await storage.put(key, value);
          if (key.startsWith(failAt)) throw Error('injected storage failure');
        },
        transaction: (action) => storage.transaction((txn) => action(failing(txn))),
      });
      try {
        await new Ledger(failAt ? failing(this.ctx.storage) : this.ctx.storage).reserve(record, {
          cfg: { ...configuration(this.env), profile: record.profile ?? 'check', ...config },
          now: record.started,
          cost,
          credit,
          retryNonce,
        });
        return Response.json({ state: 'RESERVED' });
      } catch (error) {
        return Response.json(
          { state: error.state ?? 'ERROR', reason: error.reason, budget: error.budget },
          { status: error.status ?? 500 },
        );
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
