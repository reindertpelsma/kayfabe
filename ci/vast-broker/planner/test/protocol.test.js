import test from 'node:test';
import assert from 'node:assert/strict';
import { PlanError, checkSchema } from '../index.js';
import {
  artifactReady,
  artifactGone,
  artifactStatus,
  assignmentResponse,
  startAssignment,
  endAssignment,
  verifyArtifact,
  directAssignment,
} from '../artifacts.js';
import { protocolFixture } from './protocol-fixture.js';
const rejects = (fn) => assert.throws(fn, (e) => e instanceof PlanError);
// FALSIFIER: untrusted endpoint data changes identity, admits a foreign URL,
// resurrects a gone artifact, rents twice, or removes local-build fallback.
for (const n of [1, 2]) {
  test('artifact protocol ready claims pending consumers once ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady(f.ctx, f.state, f.body, f.now);
    assert.ok(r.start_requests.includes(f.consumer.id));
    assert.equal(
      r.state.assignments.find((a) => a.assignment_id === f.consumer.id).status,
      'starting',
    );
    const again = artifactReady(f.ctx, r.state, f.body, f.now);
    assert.deepEqual(again.start_requests, []);
    assert.equal(again.state.artifacts[0].ready_minute, r.state.artifacts[0].ready_minute);
    checkSchema('protocol_state', r.state);
  });
  for (const bad of [
    'host',
    'port',
    'requested',
    'stale',
    'token',
    'instance',
    'name',
    'size',
    'scheme',
    'path',
    'sha',
  ])
    test('artifact protocol refuses ' + bad + ' ' + n, () => {
      const f = protocolFixture(n),
        b = { ...f.body },
        c = structuredClone(f.ctx);
      if (bad === 'host') b.url = b.url.replace('203.0.113.10', '203.0.113.11');
      if (bad === 'port') b.url = b.url.replace('45321', '45322');
      if (bad === 'requested') c.facts.requested_ports = [];
      if (bad === 'stale') c.facts.observed_minute = f.now - 2;
      if (bad === 'token') c.auth.verified = false;
      if (bad === 'instance') c.facts.instance_id = 'foreign-instance';
      if (bad === 'name') {
        b.artifact = 'not-in-plan';
        b.size = 4;
      }
      if (bad === 'size') b.size = 99_000_000_000;
      if (bad === 'scheme') b.url = b.url.replace('http:', 'ftp:');
      if (bad === 'path') b.url = b.url.replace('/abcdefgh/', '/short/');
      if (bad === 'sha') b.sha256 = 'not-a-sha';
      rejects(() => artifactReady(c, f.state, b, f.now));
    });
  test('artifact protocol consumer polls ready then source loss yields local fallback ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady(f.ctx, f.state, f.body, f.now);
    let state = startAssignment(r.state, f.consumer.id, 'consumer-instance', f.now);
    const c = f.consumerContext();
    const ready = artifactStatus(c, state, { version: 1, name: 'open_image' }, f.now);
    assert.equal(ready.state, 'ready');
    assert.equal(ready.sha256, f.body.sha256);
    state = endAssignment(state, f.source.id, f.now + 0.01);
    assert.equal(state.artifacts[0].state, 'gone');
    const gone = artifactStatus(c, state, { version: 1, name: 'open_image' }, f.now + 0.01);
    assert.equal(gone.state, 'gone');
    assert.equal(gone.fallback, 'build_local');
    assert.equal(gone.url, null);
    rejects(() => artifactReady(f.ctx, state, f.body, f.now + 0.01));
  });
  test('artifact protocol consumer deadline gives local fallback ' + n, () => {
    const f = protocolFixture(n);
    let state = startAssignment(f.state, f.consumer.id, 'consumer-instance', f.now);
    const c = f.consumerContext(),
      at = f.hint.deadline_minute + 0.01;
    const r = artifactStatus(c, state, { version: 1, name: 'open_image' }, at);
    assert.equal(r.state, 'gone');
    assert.equal(r.fallback, 'build_local');
    assert.equal(r.url, null);
  });
  test('artifact protocol hash mismatch invalidates direct source ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady(f.ctx, f.state, f.body, f.now);
    const state = startAssignment(r.state, f.consumer.id, 'consumer-instance', f.now),
      c = f.consumerContext();
    assert.equal(
      verifyArtifact(f.body, { sha256: 'b'.repeat(64), size: f.body.size }).reason,
      'hash_mismatch',
    );
    const gone = artifactGone(
      c,
      state,
      {
        version: 1,
        artifact: 'open_image',
        reason: 'hash_mismatch',
        observed_sha256: 'b'.repeat(64),
        observed_size: f.body.size,
      },
      f.now,
    );
    assert.equal(gone.artifacts[0].state, 'gone');
    assert.equal(
      artifactStatus(c, gone, { version: 1, name: 'open_image' }, f.now).fallback,
      'build_local',
    );
    rejects(() => artifactReady(f.ctx, gone, f.body, f.now));
  });
  test('artifact protocol assignment launch modes retain one cold executor shape ' + n, () => {
    for (const mode of ['worker_vast', 'worker_daemon', 'direct']) {
      const f = protocolFixture(n, mode === 'worker_vast' ? 'vast' : 'local');
      const state = startAssignment(f.state, f.consumer.id, 'consumer-instance', f.now);
      const out = assignmentResponse(f.consumerContext(), state, mode, f.now);
      assert.equal(out.launch.mode, mode);
      assert.deepEqual(out.assignment, f.consumer);
      checkSchema('assignment_response', out);
      if (mode === 'direct') {
        assert.equal(out.launch.worker_url, null);
        assert.equal(out.launch.report_path, 'report.json');
        assert.ok(out.artifacts.every((a) => a.state === 'gone'));
      }
    }
  });
  test('artifact protocol cannot extend identity or artifact lifetime ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady(f.ctx, f.state, f.body, f.now);
    const b = { ...f.body, sha256: 'c'.repeat(64) };
    rejects(() => artifactReady(f.ctx, r.state, b, f.now));
    const c = structuredClone(f.ctx);
    c.auth.session_id = 'other-release';
    rejects(() => artifactReady(c, r.state, f.body, f.now));
  });
}
for (const n of [1, 2]) {
  test('artifact protocol local assignment needs no Worker token ' + n, () => {
    const f = protocolFixture(n);
    const out = directAssignment(f.input, f.consumer, { plan: f.plan });
    assert.equal(out.launch.mode, 'direct');
    assert.equal(out.launch.worker_url, null);
    assert.equal(out.launch.report_path, 'report.json');
    assert.deepEqual(out.assignment, f.consumer);
    assert.ok(out.artifacts.every((x) => x.state === 'gone' && x.fallback === 'build_local'));
  });
  test('artifact protocol refuses clock skew and expired token ' + n, () => {
    const f = protocolFixture(n),
      c = structuredClone(f.ctx);
    c.auth.expires_minute = f.now;
    rejects(() => artifactReady(c, f.state, f.body, f.now));
    const s = { ...f.state, now_minute: f.now + 1 };
    rejects(() => artifactReady(f.ctx, s, f.body, f.now));
  });
  test('artifact protocol no new starts while admission is closed ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady({ ...f.ctx, admission_open: false }, f.state, f.body, f.now);
    assert.deepEqual(r.start_requests, []);
    assert.equal(r.state.artifacts[0].state, 'ready');
    assert.equal(
      r.state.assignments.find((x) => x.assignment_id === f.consumer.id).status,
      'pending',
    );
  });
  test('artifact protocol never claims more than live concurrency ' + n, () => {
    const f = protocolFixture(n);
    const blocker = f.plan.assignments.find((a) => a.id !== f.source.id && a.id !== f.consumer.id);
    const s = startAssignment(f.state, blocker.id, 'blocker-instance', f.source.start_minute);
    const r = artifactReady(f.ctx, s, f.body, f.now);
    assert.deepEqual(r.start_requests, []);
  });
  test('artifact protocol early consumer uses an actual-start wait budget ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady(f.ctx, f.state, f.body, f.now);
    const s = startAssignment(r.state, f.consumer.id, 'consumer-instance', f.now);
    const out = artifactStatus(f.consumerContext(), s, { version: 1, name: 'open_image' }, f.now);
    assert.ok(out.deadline_minute <= f.now + f.input.limits.seeding.max_wait_minutes);
    assert.ok(out.fetch_until_minute <= f.source.seed_server.serve_until_minute);
  });
  test('artifact protocol gone post creates a terminal tombstone ' + n, () => {
    const f = protocolFixture(n),
      s = artifactGone(
        f.ctx,
        f.state,
        { version: 1, artifact: 'open_image', reason: 'unavailable' },
        f.now,
      );
    assert.equal(s.artifacts[0].state, 'gone');
    rejects(() => artifactReady(f.ctx, s, f.body, f.now));
  });
  test('artifact protocol token cannot name another assignment ' + n, () => {
    const f = protocolFixture(n),
      c = structuredClone(f.ctx);
    c.auth.assignment_id = f.consumer.id;
    rejects(() => artifactReady(c, f.state, f.body, f.now));
  });
  test('artifact protocol metadata verifier checks hash and size ' + n, () => {
    const f = protocolFixture(n);
    assert.deepEqual(verifyArtifact(f.body, f.body), { valid: true, reason: null });
    assert.equal(
      verifyArtifact(f.body, { ...f.body, size: f.body.size - 1 }).reason,
      'size_mismatch',
    );
    assert.equal(
      verifyArtifact(f.body, { ...f.body, sha256: 'b'.repeat(64) }).reason,
      'hash_mismatch',
    );
  });
}
for (const n of [1, 2]) {
  test('artifact protocol wire states reject contradictory metadata ' + n, () => {
    rejects(() =>
      checkSchema('artifact_response', {
        version: 1,
        artifact: 'open_image',
        scope_key: 'open_image@box',
        state: 'ready',
        url: null,
        sha256: null,
        size: null,
        deadline_minute: 10,
        fetch_until_minute: 11,
        fallback: 'build_local',
      }),
    );
    rejects(() =>
      checkSchema('launch', {
        mode: 'direct',
        worker_url: 'https://worker.example',
        report_path: null,
      }),
    );
  });
  test('artifact protocol response does not expose foreign artifact names ' + n, () => {
    const f = protocolFixture(n),
      s = startAssignment(f.state, f.consumer.id, 'consumer-instance', f.now);
    rejects(() =>
      artifactStatus(f.consumerContext(), s, { version: 1, name: 'foreign-image' }, f.now),
    );
  });
}
for (const n of [1, 2]) {
  test('artifact protocol ended consumer is never rented again by a duplicate ready ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady(f.ctx, f.state, f.body, f.now);
    let state = startAssignment(r.state, f.consumer.id, 'consumer-instance', f.now);
    state = endAssignment(state, f.consumer.id, f.now + 0.01);
    const again = artifactReady(f.ctx, state, f.body, f.now + 0.01);
    assert.ok(!again.start_requests.includes(f.consumer.id));
    assert.equal(
      again.state.assignments.find((x) => x.assignment_id === f.consumer.id).status,
      'ended',
    );
  });
  test('artifact protocol false mismatch report cannot invalidate a good artifact ' + n, () => {
    const f = protocolFixture(n),
      r = artifactReady(f.ctx, f.state, f.body, f.now);
    const state = startAssignment(r.state, f.consumer.id, 'consumer-instance', f.now);
    rejects(() =>
      artifactGone(
        f.consumerContext(),
        state,
        {
          version: 1,
          artifact: 'open_image',
          reason: 'hash_mismatch',
          observed_sha256: f.body.sha256,
          observed_size: f.body.size,
        },
        f.now,
      ),
    );
  });
}
for (const n of [1, 2]) {
  test('artifact protocol launch mode must match provider ' + n, () => {
    const f = protocolFixture(n, 'local'),
      s = startAssignment(f.state, f.consumer.id, 'consumer-instance', f.now);
    rejects(() => assignmentResponse(f.consumerContext(), s, 'worker_vast', f.now));
    const v = protocolFixture(n, 'vast'),
      t = startAssignment(v.state, v.consumer.id, 'consumer-instance', v.now);
    rejects(() => assignmentResponse(v.consumerContext(), t, 'worker_daemon', v.now));
  });
  test('artifact protocol missing mapped ports declines serving ' + n, () => {
    const f = protocolFixture(n),
      c = structuredClone(f.ctx);
    c.facts.mapped_ports = [];
    rejects(() => artifactReady(c, f.state, f.body, f.now));
  });
}
for (const n of [1, 2])
  test('artifact protocol refuses nonpublic provider addresses ' + n, () => {
    const f = protocolFixture(n),
      c = structuredClone(f.ctx);
    c.facts.public_ip = '127.0.0.1';
    rejects(() =>
      artifactReady(
        c,
        f.state,
        { ...f.body, url: f.body.url.replace('203.0.113.10', '127.0.0.1') },
        f.now,
      ),
    );
  });
for (const n of [1, 2])
  test('artifact protocol refuses bytes above the priced allowance ' + n, () => {
    const f = protocolFixture(n);
    rejects(() => artifactReady(f.ctx, f.state, { ...f.body, size: 13_500_000_000 + n }, f.now));
  });
for (const n of [1, 2]) {
  test('artifact protocol late ready does not start a pending consumer ' + n, () => {
    const f = protocolFixture(n),
      now = f.hint.deadline_minute + 0.01;
    const c = structuredClone(f.ctx);
    c.facts.observed_minute = now;
    const r = artifactReady(c, f.state, f.body, now);
    assert.ok(!r.start_requests.includes(f.consumer.id));
  });
  test('artifact protocol source cannot publish after serve-until ' + n, () => {
    const f = protocolFixture(n),
      now = f.source.seed_server.serve_until_minute + 0.01;
    const c = structuredClone(f.ctx);
    c.facts.observed_minute = now;
    rejects(() => artifactReady(c, f.state, f.body, now));
  });
  test('artifact protocol starting claims consume a live slot ' + n, () => {
    const f = protocolFixture(2),
      first = artifactReady(f.ctx, f.state, f.body, f.now);
    assert.equal(first.start_requests.length, 1);
    const again = artifactReady(f.ctx, first.state, f.body, f.now);
    assert.deepEqual(again.start_requests, []);
    assert.equal(
      again.state.assignments.filter((a) => ['starting', 'running'].includes(a.status)).length,
      2,
    );
  });
}
