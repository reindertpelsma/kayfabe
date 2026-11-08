import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { protocolFixture } from './protocol-fixture.js';
import {
  createSession,
  startAssignment,
  endAssignment,
  artifactReady,
  artifactGone,
  artifactStatus,
  verifyArtifact,
} from '../artifacts.js';
import { MockCloud, MemoryStorage, clock } from '../../test/helpers.js';
import { Ledger } from '../../src/ledger.js';
import { Vast } from '../../src/vast.js';
import { IMAGE } from '../../src/policy.js';
import { sha256Hex } from '../../src/tokens.js';
// These are future-handler simulations, not an implementation of Worker routes.
// Only the existing MockCloud transport is called; port facts are normalized fixtures.
async function harness(consumers = 1) {
  const f = protocolFixture(consumers, 'local', {
      sha256: createHash('sha256').update('GOOD').digest('hex'),
      size: 4,
    }),
    cloud = new MockCloud(),
    storage = new MemoryStorage(),
    ledger = new Ledger(storage);
  cloud.contracts = [900, 901, 902, 903];
  cloud.extraOffers = f.plan.assignments.map((a, i) => {
    const b = f.input.boxes.find((b) => b.id === a.box_id),
      p = b.pricing;
    return {
      ...cloud.offer,
      id: 111 + i,
      machine_id: 1000 + i,
      storage_cost: p.storage_usd_gb_month,
      inet_down_cost: p.inet_down_usd_gb,
      inet_up_cost: p.inet_up_usd_gb,
      price:
        p.compute_usd_hour +
        (p.storage_usd_gb_month * a.pricing.disk_gb) / f.input.limits.storage_month_hours,
    };
  });
  const client = new Vast(cloud.fetch, {}, 'mock-artifact-simulation'),
    owned = new Set(),
    records = new Map();
  let state = createSession(f.plan, 'release-test');
  async function rent(a, at) {
    assert.ok(!records.has(a.id), 'one rental per assignment/machine');
    assert.ok(!f.input.limits.bad_machine_ids.includes(a.machine_id));
    assert.ok(!f.input.limits.bad_host_ids.includes(a.host_id));
    assert.ok(
      [...records.keys(), a.id].reduce(
        (sum, id) => sum + f.plan.assignments.find((x) => x.id === id).pricing.reserved_usd,
        0,
      ) <= f.input.limits.cost_cap_usd,
    );
    const key = String(8000 + records.size) + '-1',
      script = 'mock-assignment:' + a.id;
    const r = {
      key,
      run: key.split('-')[0],
      attempt: '1',
      sha: f.input.source_revision,
      label: 'kf-ci-' + key,
      scriptHash: await sha256Hex(script),
      started: clock + at * 60000,
      ended: null,
      id: null,
      idSource: 'create',
      profile: 'check',
      budgetClass: 'release',
      price: a.pricing.hourly_usd,
      computeStoragePrice: a.pricing.hourly_usd,
      bandwidthCost: a.pricing.transfer_usd,
      ttl: a.reservation_minutes * 60000,
      billingTtl: a.reservation_minutes * 60000,
    };
    await ledger.save(r);
    await storage.put('open:' + r.key, true);
    const response = await cloud.fetch(
      'https://console.vast.ai/api/v0/asks/' +
        (111 + f.plan.assignments.findIndex((x) => x.id === a.id)) +
        '/',
      {
        method: 'PUT',
        body: JSON.stringify({
          image: IMAGE,
          vm: true,
          disk: a.pricing.disk_gb,
          label: r.label,
          onstart: script,
        }),
      },
    );
    r.id = String((await response.json()).new_contract);
    owned.add(r.id);
    records.set(a.id, r);
    await ledger.save(r);
    const instance = cloud.instances.get(r.id);
    assert.ok(
      Math.abs(
        instance.dph_base +
          (instance.storage_cost * a.pricing.disk_gb) / 720 -
          a.pricing.hourly_usd,
      ) <= 0.000002,
      'live normalized provider quote matches reserved hourly cost',
    );
    const profile = f.input.catalog.profiles.find((p) => p.tier === a.tier);
    assert.ok(
      instance.inet_down_cost <= profile.max_inet_usd_gb &&
        instance.inet_up_cost <= profile.max_inet_usd_gb,
    );
    instance.public_ipaddr = '8.8.8.8';
    instance.ports = { '8080/tcp': [{ HostIp: '0.0.0.0', HostPort: '45321' }] };
    state = startAssignment(state, a.id, r.id, at);
    await storage.put('release-protocol-state', state);
    assert.ok((await ledger.openRecords()).length <= f.input.limits.max_concurrent_boxes);
    return r;
  }
  function ctx(a, at) {
    const r = records.get(a.id),
      base = f.ctx;
    return {
      ...base,
      auth: { ...base.auth, assignment_id: a.id, instance_id: r.id },
      facts: {
        ...base.facts,
        assignment_id: a.id,
        instance_id: r.id,
        provider: a.provider,
        observed_minute: at,
      },
    };
  }
  async function ready(body, at) {
    const current = await client.inspect(records.get(f.source.id));
    assert.ok(current);
    const c = ctx(f.source, at);
    c.facts.public_ip = current.public_ipaddr;
    c.facts.mapped_ports = current.ports['8080/tcp'].map((p) => ({
      container_port: 8080,
      public_port: Number(p.HostPort),
      protocol: 'tcp',
    }));
    const next = artifactReady(c, state, body, at);
    state = next.state;
    await storage.put('release-protocol-state', state);
    // The same request handler performs the creates; no cron is called.
    for (const id of next.start_requests)
      await rent(
        f.plan.assignments.find((a) => a.id === id),
        at,
      );
    return next;
  }
  async function close(at) {
    for (const a of f.plan.assignments) {
      const r = records.get(a.id);
      if (r) {
        await client.destroy(r);
        r.closed = 'STOPPED';
        await ledger.settle(r, clock + at * 60000);
      }
      state = endAssignment(state, a.id, at);
    }
    await storage.put('release-protocol-state', state);
    assert.equal((await ledger.openRecords()).length, 0);
    assert.equal(cloud.instances.size, 2, 'foreign fixture instances survive');
    assert.ok(cloud.deletes.every((id) => owned.has(id)));
    assert.ok([...owned].every((id) => !cloud.instances.has(id)));
    assert.ok([...records.values()].every((r) => r.ended !== null && r.verdict !== 'PASS'));
    assert.ok(state.artifacts.every((a) => a.state === 'gone'));
    assert.deepEqual(
      await storage.get('release-protocol-state'),
      state,
      'persisted registry follows every transition',
    );
  }
  return {
    f,
    cloud,
    storage,
    ledger,
    records,
    client,
    rent,
    ctx,
    ready,
    close,
    get state() {
      return state;
    },
    async end(id, at) {
      state = endAssignment(state, id, at);
      await storage.put('release-protocol-state', state);
    },
    async bad(c, body, at) {
      state = artifactGone(c, state, body, at);
      await storage.put('release-protocol-state', state);
    },
  };
}
for (const scenario of [
  'ready twice',
  'seeder dies mid-download',
  'consumer deadline passes',
  'URL host mismatch',
  'mapped port mismatch',
  'hash mismatch',
  'size mismatch',
])
  test('artifact transport simulation: ' + scenario, async () => {
    const h = await harness(),
      { f } = h;
    await h.rent(f.source, f.source.start_minute);
    const data = Buffer.from('GOOD'),
      sha = createHash('sha256').update(data).digest('hex');
    const body = { ...f.body, sha256: sha, size: data.length };
    let last = f.now;
    try {
      if (scenario === 'URL host mismatch' || scenario === 'mapped port mismatch') {
        const bad = {
          ...body,
          url:
            scenario === 'URL host mismatch'
              ? body.url.replace('8.8.8.8', '8.8.4.4')
              : body.url.replace('45321', '45322'),
        };
        await assert.rejects(() => h.ready(bad, f.now));
        assert.equal(h.cloud.creates, 1);
        return;
      }
      if (scenario === 'consumer deadline passes') {
        await h.rent(f.consumer, f.now);
        last = f.hint.deadline_minute + 0.01;
        const response = artifactStatus(
          h.ctx(f.consumer, last),
          h.state,
          { version: 1, name: 'open_image' },
          last,
        );
        assert.equal(response.state, 'gone');
        assert.equal(response.fallback, 'build_local');
        await h.ready(body, last);
        assert.equal(h.cloud.creates, 2);
        return;
      }
      await h.ready(body, f.now);
      assert.equal(h.cloud.creates, 2, 'ready POST started consumer immediately');
      const c = h.ctx(f.consumer, f.now),
        response = artifactStatus(c, h.state, { version: 1, name: 'open_image' }, f.now);
      assert.equal(response.state, 'ready');
      assert.equal(response.sha256, sha);
      if (scenario === 'ready twice') {
        await h.ready(body, f.now);
        assert.equal(h.cloud.creates, 2);
        return;
      }
      if (scenario === 'seeder dies mid-download') {
        h.cloud.instances.delete(h.records.get(f.source.id).id);
        last = f.now + 0.01;
        assert.equal(await h.client.inspect(h.records.get(f.source.id)), null);
        await h.end(f.source.id, last);
        const gone = artifactStatus(
          h.ctx(f.consumer, last),
          h.state,
          { version: 1, name: 'open_image' },
          last,
        );
        assert.equal(gone.state, 'gone');
        assert.equal(gone.fallback, 'build_local');
        assert.equal(verifyArtifact(response, { size: 1, sha256: '0'.repeat(64) }).valid, false);
      } else {
        const bytes = Buffer.from(scenario === 'hash mismatch' ? 'EVIL' : 'BAD');
        const observed = {
          size: bytes.length,
          sha256: createHash('sha256').update(bytes).digest('hex'),
        };
        const checked = verifyArtifact(response, observed);
        assert.equal(checked.valid, false);
        assert.equal(checked.reason, scenario.replace(' ', '_'));
        await h.bad(
          c,
          {
            version: 1,
            artifact: 'open_image',
            reason: checked.reason,
            observed_sha256: observed.sha256,
            observed_size: observed.size,
          },
          f.now,
        );
        const gone = artifactStatus(c, h.state, { version: 1, name: 'open_image' }, f.now);
        assert.equal(gone.state, 'gone');
        assert.equal(gone.fallback, 'build_local');
      }
      assert.ok(
        f.consumer.seed_hints[0].fallback_step_ids.every((id) =>
          f.consumer.runs.some((r) => r.id === id),
        ),
      );
    } finally {
      await h.close(last + 1);
    }
  });

for (const reason of ['hash_mismatch', 'size_mismatch'])
  test('artifact transport simulation: consumer cannot revoke sibling ' + reason, async () => {
    const h = await harness(2),
      { f } = h;
    let last = f.now;
    try {
      await h.rent(f.source, f.source.start_minute);
      await h.ready(f.body, f.now);
      const other = f.plan.assignments.find(
        (a) =>
          a.id !== f.consumer.id &&
          a.seed_hints.some(
            (x) => x.artifact === 'open_image' && x.producer_assignment_id === f.source.id,
          ),
      );
      const a = h.ctx(f.consumer, f.now);
      await h.bad(
        a,
        {
          version: 1,
          artifact: 'open_image',
          reason,
          observed_sha256: '0'.repeat(64),
          observed_size: f.body.size - 1,
        },
        f.now,
      );
      assert.equal(h.state.artifacts[0].state, 'ready');
      assert.equal(
        artifactStatus(a, h.state, { version: 1, name: 'open_image' }, f.now).state,
        'gone',
      );
      // End the reporter's exact rental before admitting its sibling.
      const record = h.records.get(f.consumer.id);
      await h.client.destroy(record);
      record.closed = 'STOPPED';
      await h.ledger.settle(record, clock + f.now * 60000);
      await h.end(f.consumer.id, f.now);
      await h.ready(f.body, f.now);
      assert.ok(h.records.has(other.id));
      const sibling = artifactStatus(
        h.ctx(other, f.now),
        h.state,
        { version: 1, name: 'open_image' },
        f.now,
      );
      assert.equal(sibling.state, 'ready');
      assert.equal(sibling.sha256, f.body.sha256);
      assert.deepEqual(await h.storage.get('release-protocol-state'), h.state);
    } finally {
      await h.close(last);
    }
  });
