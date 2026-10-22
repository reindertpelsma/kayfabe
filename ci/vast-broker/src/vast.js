// prettier-ignore
import {fetchJson,fetchBounded,readBounded,refuse,IMAGE} from './policy.js';
// prettier-ignore
import {sha256Hex} from './tokens.js';
export class Vast {
  constructor(fetcher, cfg, key) {
    this.fetcher = fetcher;
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
  async create(offer, record, script) {
    const body = {
      image: IMAGE,
      vm: true,
      disk: 120,
      runtype: 'ssh_direct',
      onstart: script,
      label: record.label,
      target_state: 'running',
      cancel_unavail: true,
    };
    const response = await this.fetcher('https://console.vast.ai/api/v0/asks/' + offer.id + '/', {
      method: 'PUT',
      headers: this.headers,
      body: JSON.stringify(body),
      redirect: 'manual',
      signal: AbortSignal.timeout(8000),
    });
    let r;
    try {
      r = JSON.parse(
        new TextDecoder('utf-8', { fatal: true }).decode(await readBounded(response, 262144)),
      );
    } catch {
      refuse();
    }
    if (
      !response.redirected &&
      [200, 400, 404, 410].includes(response.status) &&
      r.success === false &&
      r.new_contract === undefined
    )
      refuse('CREATE_REFUSED', 503);
    if (r.success !== true || !Number.isSafeInteger(r.new_contract) || r.new_contract <= 0)
      refuse();
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
      i.label !== record.label
    )
      refuse('OWNERSHIP_MISMATCH', 409);
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
