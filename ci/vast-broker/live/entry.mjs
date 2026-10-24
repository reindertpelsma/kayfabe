// Local diagnostic entrypoint, copied beside a private source overlay by smoke-live.py.
// Never deploy: production uses src/index.js and the committed GPU policy/workload.
import worker, { createService } from './src/index.js';
export class BrokerLedger {
  constructor(ctx, env) {
    this.storage = ctx.storage;
    this.env = env;
    let offers = [];
    const emit = async (value) => {
      const r = await fetch('http://127.0.0.1:18792/event', {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + env.OWNER_STOP_TOKEN,
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(value),
      });
      if (!r.ok) throw Error('DIAGNOSTIC_SINK');
    };
    const tracked = async (url, init = {}) => {
      const u = new URL(url);
      if (u.hostname === 'api.github.com')
        return fetch('http://127.0.0.1:18788' + u.pathname + u.search, init);
      if (u.hostname !== 'console.vast.ai') return fetch(url, init);
      const create = init.method === 'PUT' && u.pathname.includes('/asks/');
      let body;
      if (create) {
        body = JSON.parse(init.body);
        const offer = offers.find((o) => String(o.id) === u.pathname.split('/').at(-2));
        if (!offer || offer.dph_total > 0.8) throw Error('LOCAL_PRICE_CAP');
        await emit({
          kind: 'intent',
          label: body.label,
          price: offer.dph_total,
          gpu: offer.gpu_name,
          runtype: body.runtype,
          vm: body.vm,
        });
      }
      const r = await fetch(url, init),
        data = await r.clone().json();
      if (u.pathname === '/api/v0/bundles/') offers = data.offers ?? [];
      if (create && data.success === true)
        await emit({ kind: 'created', id: data.new_contract, label: body.label });
      if (/^\/api\/v0\/instances\/\d+\/$/.test(u.pathname)) {
        const i = data.instances;
        await emit({
          kind: 'provider',
          method: init.method ?? 'GET',
          status: r.status,
          actual_status_present: Object.hasOwn(i ?? {}, 'actual_status'),
          actual_status: i?.actual_status ?? null,
          cur_state: i?.cur_state ?? null,
          intended_status: i?.intended_status ?? null,
          absent: i === null || r.status === 404,
        });
      }
      return r;
    };
    this.service = createService({ env, storage: ctx.storage, fetch: tracked });
  }
  async fetch(request) {
    if (new URL(request.url).origin === 'https://internal.invalid') {
      await this.service.sweep();
      return new Response('ok');
    }
    return this.service.fetch(request);
  }
}
export default worker;
