import test from 'node:test';
import { createHash } from 'node:crypto';
import assert from 'node:assert/strict';
import { protocolFixture } from './protocol-fixture.js';
import { fixture } from './helpers.js';
import { plan } from '../index.js';
import {
  artifactReady,
  artifactGone,
  artifactStatus,
  startAssignment,
  endAssignment,
  verifyArtifact,
} from '../artifacts.js';
// FALSIFIER: a lying consumer revokes a sibling's export, an equivalent nonpublic
// endpoint is admitted, or self-reported bytes can skip an unpinned local build.
for (const reason of ['hash_mismatch', 'size_mismatch']) {
  test('RV310 consumer isolation ' + reason, () => {
    const f = protocolFixture(2),
      other = f.plan.assignments.find(
        (a) =>
          a.id !== f.consumer.id &&
          a.seed_hints.some(
            (h) => h.producer_assignment_id === f.source.id && h.artifact === 'open_image',
          ),
      );
    let state = artifactReady(f.ctx, f.state, f.body, f.now).state;
    state = startAssignment(state, f.consumer.id, 'consumer-instance', f.now);
    const original = structuredClone(state.artifacts);
    state = artifactGone(
      f.consumerContext(),
      state,
      {
        version: 1,
        artifact: 'open_image',
        reason,
        observed_sha256: 'b'.repeat(64),
        observed_size: f.body.size - 1,
      },
      f.now,
    );
    assert.deepEqual(state.artifacts, original);
    assert.equal(
      artifactStatus(f.consumerContext(), state, { version: 1, name: 'open_image' }, f.now).state,
      'gone',
    );
    state = endAssignment(state, f.consumer.id, f.now);
    state = startAssignment(state, other.id, 'other-instance', f.now);
    const ctx = {
      ...f.ctx,
      auth: { ...f.ctx.auth, assignment_id: other.id, instance_id: 'other-instance' },
    };
    const response = artifactStatus(ctx, state, { version: 1, name: 'open_image' }, f.now);
    assert.equal(response.state, 'ready');
    assert.equal(response.sha256, f.body.sha256);
    assert.doesNotThrow(() => artifactReady(f.ctx, state, f.body, f.now));
  });
}
for (const ip of [
  '::ffff:7f00:1',
  '::ffff:a00:1',
  '::ffff:a9fe:a9fe',
  '::ffff:c0a8:101',
  '::ffff:ac10:1',
  '::ffff:6440:1',
  'ff02::1',
  'ff0e::1',
  'fec0::1',
  'fe80::1',
  'fc00::1',
  '::',
  '::1',
  '100.64.0.1',
  '100.127.255.254',
  '192.0.2.1',
  '203.0.113.1',
  '198.51.100.1',
  '198.18.0.1',
  '0.0.0.1',
  '224.0.0.1',
  '240.0.0.1',
  '2001:db8::1',
  '3fff::1',
  '2002:7f00:1::',
])
  test('RV310 endpoint refuses ' + ip, () => {
    const f = protocolFixture(),
      ctx = structuredClone(f.ctx);
    ctx.facts.public_ip = ip;
    const host = ip.includes(':') ? '[' + ip + ']' : ip;
    assert.throws(
      () =>
        artifactReady(
          ctx,
          f.state,
          { ...f.body, url: 'http://' + host + ':45321/abcdefgh/open-image.tar.zst' },
          f.now,
        ),
      (e) => e.code === 'ARTIFACT_IP',
    );
  });
for (const ip of [
  '8.8.8.8',
  '1.1.1.1',
  '2606:4700:4700::1111',
  '2001:4860:4860::8888',
  '::ffff:808:808',
])
  test('RV310 endpoint accepts public unicast ' + ip, () => {
    const f = protocolFixture(),
      ctx = structuredClone(f.ctx);
    ctx.facts.public_ip = ip;
    const host = ip.includes(':') ? '[' + ip + ']' : ip;
    assert.equal(
      artifactReady(
        ctx,
        f.state,
        { ...f.body, url: 'http://' + host + ':45321/abcdefgh/open-image.tar.zst' },
        f.now,
      ).state.artifacts[0].state,
      'ready',
    );
  });
for (const artifact of ['open_image', 'kf3_bins'])
  test('RV310 local build required without trusted content ' + artifact, () => {
    const f = protocolFixture(2).input;
    const spec = f.catalog.artifacts.find((a) => a.id === artifact);
    spec.reuse_policy = 'local_build';
    spec.trusted_content = null;
    const p = plan(f);
    assert.ok(p.assignments.every((a) => a.seed_hints.every((h) => h.artifact !== artifact)));
    assert.ok(
      p.assignments
        .flatMap((a) => a.runs)
        .filter((r) => r.produces.some((x) => x.startsWith(artifact + '@')))
        .every((r) => r.optional_seed_for === null),
    );
  });
for (const n of [1, 2]) {
  for (const field of ['sha256', 'size'])
    test('RV310 producer cannot select trusted ' + field + ' ' + n, () => {
      const f = protocolFixture(n),
        body = { ...f.body };
      body[field] = field === 'sha256' ? 'c'.repeat(64) : body.size - 1;
      assert.throws(
        () => artifactReady(f.ctx, f.state, body, f.now),
        (e) => e.code === 'ARTIFACT_TRUST',
      );
    });
  test('RV310 assignment contains trusted content ' + n, () => {
    const f = protocolFixture(n),
      spec = f.input.catalog.artifacts.find((a) => a.id === 'open_image');
    assert.deepEqual(f.hint.trusted_content, spec.trusted_content);
    assert.equal(f.hint.trusted_content.source_revision, f.plan.source_revision);
  });
}
for (const artifact of ['open_image', 'kf3_bins']) {
  test('RV310 content revision must match ' + artifact, () => {
    const f = fixture();
    f.catalog.artifacts.find((a) => a.id === artifact).trusted_content.source_revision = 'f'.repeat(
      40,
    );
    assert.throws(
      () => plan(f),
      (e) => e.code === 'ARTIFACT_TRUST',
    );
  });
  test('RV310 trusted bytes must fit priced allowance ' + artifact, () => {
    const f = fixture(),
      spec = f.catalog.artifacts.find((a) => a.id === artifact);
    spec.trusted_content.size = spec.gb * 1e9 + 1;
    assert.throws(
      () => plan(f),
      (e) => e.code === 'ARTIFACT_TRUST',
    );
  });
}
for (const artifact of ['source', 'windows_base'])
  for (const field of ['reuse_policy', 'trusted_content'])
    test('RV310 catalog requires ' + field + ' ' + artifact, async () => {
      const { checkSchema } = await import('../index.js');
      const f = fixture();
      delete f.catalog.artifacts.find((a) => a.id === artifact)[field];
      assert.throws(() => checkSchema('catalog', f.catalog));
    });
for (const ip of [
  '::ffff:127.0.0.1',
  '::ffff:10.0.0.1',
  '2001::1',
  '2001:2::1',
  '255.255.255.255',
  '192.168.0.1',
])
  test('RV310 endpoint refuses additional special ' + ip, () => {
    const f = protocolFixture(),
      ctx = structuredClone(f.ctx);
    ctx.facts.public_ip = ip;
    assert.throws(
      () =>
        artifactReady(
          ctx,
          f.state,
          { ...f.body, url: 'http://[' + ip + ']:45321/abcdefgh/open-image.tar.zst' },
          f.now,
        ),
      (e) => e.code === 'ARTIFACT_IP',
    );
  });
for (const n of [1, 2])
  test('RV310 self-selected checksum is not provenance ' + n, () => {
    const f = protocolFixture(n),
      bytes = Buffer.from('SYNTHETIC untrusted image ' + n);
    const observed = {
      sha256: createHash('sha256').update(bytes).digest('hex'),
      size: bytes.length,
    };
    assert.equal(
      verifyArtifact(observed, observed).valid,
      true,
      'consistency alone is insufficient',
    );
    assert.equal(verifyArtifact(f.hint.trusted_content, observed).valid, false);
    assert.throws(
      () => artifactReady(f.ctx, f.state, { ...f.body, ...observed }, f.now),
      (e) => e.code === 'ARTIFACT_TRUST',
    );
  });
for (const n of [1, 2])
  test('RV310 source withdrawal before publication remains terminal ' + n, () => {
    const f = protocolFixture(n),
      state = artifactGone(
        f.ctx,
        f.state,
        { version: 1, artifact: 'open_image', reason: 'unavailable' },
        f.now,
      );
    assert.equal(state.artifacts[0].state, 'gone');
    assert.throws(
      () => artifactReady(f.ctx, state, f.body, f.now),
      (e) => e.code === 'ARTIFACT_GONE',
    );
  });
for (const reason of ['hash_mismatch', 'size_mismatch'])
  test('RV310 shared registry excludes consumer reports ' + reason, async () => {
    const { checkSchema } = await import('../index.js'),
      f = protocolFixture();
    const state = artifactReady(f.ctx, f.state, f.body, f.now).state;
    state.artifacts[0].state = 'gone';
    state.artifacts[0].gone_reason = reason;
    assert.throws(() => checkSchema('protocol_state', state));
  });
for (const reason of ['hash_mismatch', 'size_mismatch'])
  test('RV310 duplicate consumer cache refuses ' + reason, () => {
    const f = protocolFixture();
    let state = artifactReady(f.ctx, f.state, f.body, f.now).state;
    state = startAssignment(state, f.consumer.id, 'consumer-instance', f.now);
    state = artifactGone(
      f.consumerContext(),
      state,
      {
        version: 1,
        artifact: 'open_image',
        reason,
        observed_sha256: 'b'.repeat(64),
        observed_size: f.body.size - 1,
      },
      f.now,
    );
    state.consumer_failures.push({ ...state.consumer_failures[0] });
    assert.throws(
      () => artifactStatus(f.consumerContext(), state, { version: 1, name: 'open_image' }, f.now),
      (e) => e.code === 'PROTOCOL_STATE',
    );
  });
for (const field of ['consumer_assignment_id', 'producer_assignment_id'])
  test('RV310 foreign consumer cache refuses ' + field, () => {
    const f = protocolFixture();
    let state = artifactReady(f.ctx, f.state, f.body, f.now).state;
    state = startAssignment(state, f.consumer.id, 'consumer-instance', f.now);
    state = artifactGone(
      f.consumerContext(),
      state,
      {
        version: 1,
        artifact: 'open_image',
        reason: 'hash_mismatch',
        observed_sha256: 'b'.repeat(64),
        observed_size: f.body.size - 1,
      },
      f.now,
    );
    state.consumer_failures[0][field] =
      field === 'consumer_assignment_id' ? f.source.id : f.consumer.id;
    assert.throws(
      () => artifactStatus(f.consumerContext(), state, { version: 1, name: 'open_image' }, f.now),
      (e) => e.code === 'PROTOCOL_STATE',
    );
  });
