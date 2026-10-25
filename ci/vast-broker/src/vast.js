// prettier-ignore
import {fetchJson,fetchBounded,readBounded,refuse,IMAGE} from './policy.js';
// prettier-ignore
import {sha256Hex} from './tokens.js';
// Log-only trace of a failed create (owner diagnostics, never sent to the runner): the HTTP
// status, a fixed class and a short excerpt of the provider's own error text. The excerpt is
// restricted to a plain character set and dropped whole if it could carry a credential.
// Nothing else (headers, key, body sent, onstart, env, the response) is ever logged.
function providerError(r) {
  const text = r && typeof r === 'object' ? (typeof r.error === 'string' ? r.error : r.msg) : '';
  if (typeof text !== 'string') return '';
  const clean = text.replace(/[^A-Za-z0-9 _.,:;'()/=-]/g, '').slice(0, 120);
  const risky = (v) => /bearer/i.test(v) || /[A-Za-z0-9_-]{24}/.test(v);
  return risky(text) || risky(clean) ? '' : clean;
}
function logCreateFailed(status, klass, r) {
  console.log(
    JSON.stringify({
      event: 'create_failed',
      status: Number.isSafeInteger(status) ? status : null,
      class: klass,
      provider_error: providerError(r),
    }),
  );
}
export class Vast {
  constructor(fetcher, cfg, key) {
    this.fetcher = (...args) => fetcher(...args);
    this.cfg = cfg;
    this.headers = { Authorization: 'Bearer ' + key, 'Content-Type': 'application/json' };
    if (!key) refuse();
  }
  async api(path, method = 'GET', body, missing = false) {
    const url = new URL('https://console.vast.ai' + path);
    if (!url.pathname.endsWith('/')) url.pathname += '/';
    return fetchJson(
      this.fetcher,
      url.href,
      {
        method,
        headers: this.headers,
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      },
      missing,
    );
  }
  async offers() {
    // Fetch both verification classes so availability can show the impact of
    // mandatory verification. Admission always rechecks the returned fields.
    const q = {
      limit: 100,
      type: 'ondemand',
      gpu_arch: { eq: 'nvidia' },
      compute_cap: { gte: 750 },
      dph_total: { lte: Math.max(this.cfg.price, this.cfg.sweepPrice) },
      vms_enabled: { eq: true },
      rentable: { eq: true },
      rented: { eq: false },
      num_gpus: { eq: 1 },
      disk_space: { gte: 120 },
      allocated_storage: 120,
      inet_up_cost: { lte: this.cfg.maxInet },
      inet_down_cost: { lte: this.cfg.maxInet },
      order: [['dph_total', 'asc']],
    };
    const result = await this.api('/api/v0/bundles', 'POST', q);
    if (!Array.isArray(result.offers) || result.offers.length > 100) refuse();
    return result.offers;
  }
  // The live account API exposes prepaid funds as credit; balance is a distinct legacy field.
  async credit() {
    const r = await this.api('/api/v0/users/current');
    return r.credit;
  }
  async create(offer, record, script, environment = {}) {
    const body = {
      image: IMAGE,
      vm: true,
      disk: 120,
      runtype: 'ssh_direct',
      onstart: script,
      env: Object.fromEntries(
        Object.entries(environment).map(([key, value]) => {
          if (!/^KF_[A-Z_]+$/.test(key) || !/^[A-Za-z0-9_.:/-]+$/.test(String(value))) refuse();
          return [key, String(value)];
        }),
      ),
      label: record.label,
      target_state: 'running',
      cancel_unavail: true,
    };
    let response;
    try {
      response = await this.fetcher('https://console.vast.ai/api/v0/asks/' + offer.id + '/', {
        method: 'PUT',
        headers: this.headers,
        body: JSON.stringify(body),
        redirect: 'manual',
        signal: AbortSignal.timeout(8000),
      });
    } catch (error) {
      logCreateFailed(null, 'transport');
      throw error;
    }
    let r;
    try {
      r = JSON.parse(
        new TextDecoder('utf-8', { fatal: true }).decode(await readBounded(response, 262144)),
      );
    } catch {
      logCreateFailed(response.status, 'unparsable');
      refuse();
    }
    const ok = response.status >= 200 && response.status < 300 && !response.redirected;
    if (
      !response.redirected &&
      [200, 400, 404, 410].includes(response.status) &&
      r.success === false &&
      r.new_contract === undefined
    ) {
      logCreateFailed(response.status, 'success_false', r);
      refuse('CREATE_REFUSED', 503);
    }
    if (r.success !== true || !Number.isSafeInteger(r.new_contract) || r.new_contract <= 0) {
      const klass = !ok ? 'http_refusal' : r.success === false ? 'success_false' : 'no_contract';
      logCreateFailed(response.status, klass, r);
      refuse();
    }
    return String(r.new_contract);
  }
  async reconcile(record) {
    const query = new URLSearchParams({
      limit: '2',
      select_filters: JSON.stringify({ label: { eq: record.label } }),
    });
    const r = await this.api('/api/v1/instances?' + query);
    if (r.success !== true || !Array.isArray(r.instances) || r.next_token) refuse();
    const matches = r.instances.filter((x) => x.label === record.label);
    if (matches.length === 0) return null;
    if (matches.length !== 1) refuse();
    const candidate = matches[0];
    if (
      typeof candidate.onstart !== 'string' ||
      (await sha256Hex(candidate.onstart)) !== record.scriptHash ||
      candidate.image_uuid !== IMAGE ||
      !Number.isSafeInteger(candidate.id) ||
      candidate.id <= 0
    )
      refuse();
    return String(candidate.id);
  }
  async inspect(record) {
    const response = await this.api('/api/v0/instances/' + record.id, 'GET', undefined, true);
    if (response === null) return null;
    const i = response.instances;
    if (i === null) return null;
    if (
      !i ||
      String(i.id) !== record.id ||
      !record.label.startsWith('kf-ci-') ||
      (record.idSource !== 'create' && i.label !== record.label)
    )
      refuse('OWNERSHIP_MISMATCH', 409);
    if (i.label !== record.label) console.log(JSON.stringify({ event: 'OWNERSHIP_MISMATCH' }));
    return i;
  }
  async bootLog(record) {
    const r = await this.api('/api/v0/instances/request_logs/' + record.id, 'PUT', { tail: 40 });
    if (r.success !== true || typeof r.result_url !== 'string' || r.result_url.length > 2048)
      refuse();
    let url;
    try {
      url = new URL(r.result_url);
    } catch {
      refuse();
    }
    if (
      url.protocol !== 'https:' ||
      url.hostname !== 's3.amazonaws.com' ||
      url.port ||
      url.username ||
      url.password ||
      url.hash ||
      !url.pathname.startsWith('/vast.ai/instance_logs/')
    )
      refuse();
    const data = await fetchBounded(this.fetcher, url.href, { method: 'GET' }, 65536, true);
    if (data === null) return null;
    return new TextDecoder('utf-8', { fatal: true }).decode(data);
  }
  async destroy(record) {
    const instance = await this.inspect(record);
    if (instance === null) return;
    const r = await this.api('/api/v0/instances/' + record.id, 'DELETE');
    if (r.success !== true) refuse();
    // Acknowledgement is not absence. Retain liability and retry via cron if still present.
    if ((await this.inspect(record)) !== null) refuse();
  }
}
