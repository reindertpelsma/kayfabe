import { beforeEach, describe, expect, it, vi } from 'vitest';
// Dynamic import lets each independently named case record EXPECTED RED before code exists.
let mod: any;
const publicKey = 'ssh-ed25519 ' + 'A'.repeat(68) + ' ci';
const claims = { run_id: '123', run_attempt: '1' };
const next = { run_id: '124', run_attempt: '1' };
const now = 1800000000000;
class Store {
  value: any;
  async get() { return structuredClone(this.value); }
  async put(_key: string, value: any) { this.value = structuredClone(value); }
}
class Account {
  credit = 20; price: any = 0.2; creates = 0; error = false; ambiguous = false;
  runState = 'in_progress'; deleted: string[] = []; inspected: string[] = []; attached = 0;
  instances = new Map<string, any>([
    ['777', {id: 777, label: 'owner-work'}],
    ['888', {id: 888, label: 'kf-ci-unrecorded'}],
  ]);
  offer: any = {id: 5, vms_enabled: true, verified: true, rentable: true, rented: false,
    gpu_name: 'RTX_4070', num_gpus: 1, reliability: .999, disk_space: 200};
  calls: {path: string, method: string, body: any}[] = [];
  fetch = vi.fn(async (input: any, init: any = {}) => {
    const url = new URL(String(input)); const path = url.pathname;
    const body = init.body ? JSON.parse(init.body) : undefined;
    this.calls.push({path, method: init.method || 'GET', body});
    if (url.hostname === 'api.github.com') {
      return Response.json(this.runState === 'gone' ? {} : {status: this.runState}, {status: this.runState === 'gone' ? 404 : 200});
    }
    if (this.error) return new Response('UPSTREAM-SENSITIVE-BODY', {status: 500});
    if (path === '/api/v0/users/current') return Response.json({balance: this.credit});
    if (path === '/api/v0/bundles') return Response.json({offers: [{...this.offer, dph_total: this.price}]});
    if (path === '/api/v0/asks/5') {
      this.creates++;
      this.instances.set('900', {id: 900, label: body.label, actual_status: 'loading', ssh_host: 'ssh1.vast.ai', ssh_port: 12345});
      if (this.ambiguous) throw new Error('UPSTREAM-SENSITIVE-BODY');
      return Response.json({success: true, new_contract: 900, instance_api_key: 'UPSTREAM-SENSITIVE-BODY'});
    }
    const match = path.match(/^\/api\/v0\/instances\/(\d+)(\/ssh)?$/);
    if (!match) throw new Error('Forbidden account enumeration or unexpected endpoint: ' + path);
    const id = match[1];
    if (match[2]) { this.attached++; return Response.json({success: true}); }
    if (init.method === 'DELETE') { this.deleted.push(id); this.instances.delete(id); return Response.json({success: true}); }
    this.inspected.push(id);
    return this.instances.has(id) ? Response.json({instances: this.instances.get(id)}) : new Response('', {status: 404});
  });
}
let account: Account, store: Store, broker: any;
const request = (path = '/rent', body: any = {ssh_public_key: publicKey}, method = 'POST') =>
  new Request('https://broker.example' + path, {method, ...(method === 'GET' ? {} : {body: JSON.stringify(body)})});
const rent = (identity = claims, body?: any) => broker.handle(request('/rent', body), identity);
const endpoint = (kind: string, id = '900', identity = claims) => broker.handle(request('/' + kind + '/' + id, {}, kind === 'status' ? 'GET' : 'POST'), identity);
beforeEach(async () => {
  mod = await import('../src/broker');
  account = new Account(); store = new Store();
  broker = new mod.Broker(store, account.fetch, () => now);
});
describe('fixed capability and admission', () => {
  it('returns an id without waiting for boot and never returns upstream secrets', async () => {
    const r = await rent(); expect(r.status).toBe(200); expect(await r.json()).toEqual({id: '900'});
    expect(account.inspected).toEqual([]);
  });
  it.each([7.99, -1, null, '20', Infinity])('refuses invalid or below-reserve account balance %s', async credit => {
    account.credit = credit as any; expect((await rent()).status).toBe(409); expect(account.creates).toBe(0);
  });
  it('checks the account guard even with a cheap offer', async () => {
    account.price = .001; account.credit = 0; expect((await rent()).status).toBe(409); expect(account.creates).toBe(0);
  });
  it.each([.51, -1, null, '0.2', Infinity])('refuses over-cap or invalid offer price %s', async price => {
    account.price = price; expect((await rent()).status).toBe(503); expect(account.creates).toBe(0);
  });
  it.each([{verified: false}, {vms_enabled: false}, {gpu_name: 'foreign'}, {reliability: .98},
    {num_gpus: 2}, {disk_space: 10}])('rechecks returned offer metadata %j', async patch => {
    Object.assign(account.offer, patch); expect((await rent()).status).toBe(503); expect(account.creates).toBe(0);
  });
  it('uses a fixed search, image, disk, script and label', async () => {
    await rent(); const search = account.calls.find(x => x.path.endsWith('/bundles'))!.body;
    expect(search).toMatchObject({verified: {eq: true}, vms_enabled: {eq: true}, num_gpus: {eq: 1}});
    const create = account.calls.find(x => x.path.endsWith('/asks/5'))!.body;
    expect(create).toMatchObject({disk: 120, vm: true, image: 'docker.io/vastai/kvm:ubuntu_cli_22.04-2025-05-16'});
    expect(create.label).toBe('kf-ci-123-1'); expect(create.onstart).toContain("<<'KF_CI_KEY'");
    expect(create.onstart).toContain(publicKey); expect(account.attached).toBe(1);
  });
  it.each([{image: 'evil'}, {ttl: 1}, {onstart: 'evil'}, {offer: 4}])('rejects additional client capability %j', async extra => {
    expect((await rent(claims, {ssh_public_key: publicKey, ...extra})).status).toBe(400); expect(account.creates).toBe(0);
  });
  it('refuses another live run', async () => { await rent(); expect((await rent(next)).status).toBe(409); expect(account.creates).toBe(1); });
  it('refuses another live run after a broker restart', async () => {
    await rent(); broker = new mod.Broker(store, account.fetch, () => now);
    expect((await rent(next)).status).toBe(409); expect(account.creates).toBe(1);
  });
  it('persists reservations before create and never retries an ambiguous creation', async () => {
    account.ambiguous = true; expect((await rent()).status).toBe(503);
    account.ambiguous = false; broker = new mod.Broker(store, account.fetch, () => now);
    expect((await rent()).status).toBe(409); expect((await rent(next)).status).toBe(409); expect(account.creates).toBe(1);
  });
  it('has one winner under concurrent rental requests', async () => {
    const rs = await Promise.all([rent(), rent(next)]); expect(rs.filter(r => r.status === 200)).toHaveLength(1); expect(account.creates).toBe(1);
  });
  it('caps daily rentals even after destruction', async () => {
    broker = new mod.Broker(store, account.fetch, () => now, {dailyRentals: 1});
    await rent(); await endpoint('destroy'); expect((await rent(next)).status).toBe(409); expect(account.creates).toBe(1);
  });
  it('caps full-TTL daily spend reservations', async () => {
    broker = new mod.Broker(store, account.fetch, () => now, {dailySpend: .6});
    await rent(); await endpoint('destroy'); expect((await rent(next)).status).toBe(409); expect(account.creates).toBe(1);
  });
  it('counts spending again on the next UTC day', async () => {
    broker = new mod.Broker(store, account.fetch, () => now, {dailyRentals: 1});
    await rent(); await endpoint('destroy'); broker = new mod.Broker(store, account.fetch, () => now + 86400000, {dailyRentals: 1});
    expect((await rent(next)).status).toBe(200);
  });
});
describe('SSH input validation', () => {
  it.each(["'", '"', '\n', '\r', '$()', '`id`', ';id', ' ', '\u0000'])('rejects hostile suffix %j', async suffix => {
    expect((await rent(claims, {ssh_public_key: publicKey + suffix})).status).toBe(400); expect(account.creates).toBe(0);
  });
  it.each(['ssh-rsa ' + 'A'.repeat(68), 'ssh-ed25519 ' + 'A'.repeat(67), publicKey + '\nsecond', 'ssh-ed25519 ' + 'A'.repeat(69)])('rejects malformed key %s', async key => {
    expect((await rent(claims, {ssh_public_key: key})).status).toBe(400); expect(account.creates).toBe(0);
  });
});
describe('ledger ownership and lifecycle', () => {
  it.each(['777', '888'])('never inspects or destroys foreign id %s', async id => {
    expect((await endpoint('destroy', id)).status).toBe(404); expect(account.deleted).toEqual([]); expect(account.inspected).toEqual([]);
  });
  it.each(['777', '888'])('never exposes status of foreign id %s', async id => {
    expect((await endpoint('status', id)).status).toBe(404); expect(account.inspected).toEqual([]);
  });
  it('checks the live label before deletion', async () => {
    await rent(); account.instances.get('900').label = 'owner-work'; expect((await endpoint('destroy')).status).toBe(409); expect(account.deleted).toEqual([]);
  });
  it('checks the live label before publishing SSH coordinates', async () => {
    await rent(); Object.assign(account.instances.get('900'), {label: 'foreign', actual_status: 'running'});
    expect((await endpoint('status')).status).toBe(409);
  });
  it('never lets another run destroy an owned record', async () => { await rent(); expect((await endpoint('destroy', '900', next)).status).toBe(404); expect(account.deleted).toEqual([]); });
  it('never lets another run read owned status', async () => { await rent(); expect((await endpoint('status', '900', next)).status).toBe(404); expect(account.inspected).toEqual([]); });
  it('returns coordinates only when Vast publishes running SSH status', async () => {
    await rent(); expect(await (await endpoint('status')).json()).toEqual({state: 'pending'});
    account.instances.get('900').actual_status = 'running';
    expect(await (await endpoint('status')).json()).toEqual({state: 'running', host: 'ssh1.vast.ai', port: 12345});
  });
  it.each([{ssh_host: '-oProxyCommand=evil'}, {ssh_port: '22'}, {ssh_port: 0}, {ssh_port: 65536}])('rejects unsafe coordinates %j', async patch => {
    await rent(); Object.assign(account.instances.get('900'), {actual_status: 'running', ...patch});
    expect((await endpoint('status')).status).toBe(503);
  });
  it('destroys expired owned records only and keeps tombstones', async () => {
    await rent(); broker = new mod.Broker(store, account.fetch, () => now + 151 * 60000); await broker.sweep();
    expect(account.deleted).toEqual(['900']); expect(account.instances.has('777')).toBe(true); expect(account.instances.has('888')).toBe(true);
    expect(await (await rent()).json()).toEqual({id: '900'}); expect(account.creates).toBe(1);
  });
  it.each(['completed', 'gone'])('destroys owned instance when run is %s', async state => {
    await rent(); account.runState = state; await broker.sweep(); expect(account.deleted).toEqual(['900']);
  });
  it('does not destroy before TTL when the run is active', async () => { await rent(); await broker.sweep(); expect(account.deleted).toEqual([]); });
  it('retains failed cleanup for retry', async () => {
    await rent(); account.error = true; await broker.sweep(now + 151 * 60000); expect(account.deleted).toEqual([]);
    account.error = false; await broker.sweep(now + 151 * 60000); expect(account.deleted).toEqual(['900']);
  });
  it('explicit destroy is idempotent', async () => { await rent(); await endpoint('destroy'); expect((await endpoint('destroy')).status).toBe(200); expect(account.deleted).toEqual(['900']); });
  it('returns the same id for repeated POST without second creation', async () => { await rent(); expect(await (await rent()).json()).toEqual({id: '900'}); expect(account.creates).toBe(1); });
  it('returns the same id after process restart', async () => {
    await rent(); broker = new mod.Broker(store, account.fetch, () => now); expect(await (await rent()).json()).toEqual({id: '900'}); expect(account.creates).toBe(1);
  });
  it('rejects key changes for an existing run', async () => {
    await rent(); expect((await rent(claims, {ssh_public_key: 'ssh-ed25519 ' + 'B'.repeat(68)})).status).toBe(409); expect(account.creates).toBe(1);
  });
  it('does not echo upstream bodies or exceptions', async () => {
    account.error = true; const r = await rent(); expect(r.status).toBe(503); expect(await r.text()).not.toContain('SENSITIVE');
  });
});
