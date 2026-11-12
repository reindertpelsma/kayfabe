import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  CANONICAL_REPO,
  PROBE_FILE,
  SEED_PROBE_CONTENT,
  SEED_TOKEN,
  WIRE_STAGES,
  validateDaemonFrame,
  validateServerFrame,
} from '../src/hosts.js';

// FALSIFIER: the golden fixture (copied by the daemon side) and the Worker disagree about a wire
// value or about which frame is valid.
const fx = JSON.parse(readFileSync(new URL('./fixtures/hosts-wire.json', import.meta.url), 'utf8'));

test('fixture constants equal the Worker constants', () => {
  const c = fx.constants;
  assert.equal(c.canonical_repo, CANONICAL_REPO);
  assert.deepEqual(c.stage_names, [...WIRE_STAGES]);
  assert.equal(c.probe_content, SEED_PROBE_CONTENT);
  assert.equal(c.probe_path, '/<token>/' + PROBE_FILE);
  assert.equal(new RegExp(c.seed_token_regex).source, SEED_TOKEN.source);
  assert.equal(c.deadline_field, 'deadline_ms');
  assert.match(c.compute_cap_example, /^[0-9]{1,2}\.[0-9]$/);
});
for (const [direction, validate] of [
  ['daemon_to_worker', validateDaemonFrame],
  ['worker_to_daemon', validateServerFrame],
])
  for (const [type, { valid, invalid }] of Object.entries(fx[direction])) {
    test(direction + ' ' + type + ': every valid frame is accepted', () => {
      for (const frame of valid) assert.equal(validate(frame), null, JSON.stringify(frame));
    });
    test(direction + ' ' + type + ': every invalid frame is refused', () => {
      for (const { frame, why } of invalid) assert.notEqual(validate(frame), null, why);
    });
  }
test('fixture covers every frame type the validators know', () => {
  assert.deepEqual(Object.keys(fx.daemon_to_worker).sort(), [
    'ack',
    'cmd_done',
    'cmd_output',
    'heartbeat',
    'hello',
    'nack',
    'seed_register',
    'unknown',
  ]);
  assert.deepEqual(Object.keys(fx.worker_to_daemon).sort(), [
    'cancel',
    'cmd',
    'error',
    'offer',
    'seed_ack',
    'welcome',
  ]);
});

import { buildReport, cleanClaimsReport } from '../src/runreport.js';
test('run report: tree_key_mismatch is kept when boolean, dropped otherwise, never touches the verdict', () => {
  const f = fx.run_report_fields.tree_key_mismatch;
  for (const v of f.valid)
    assert.equal(cleanClaimsReport({ schema: 1, tree_key_mismatch: v }).tree_key_mismatch, v);
  for (const v of f.invalid)
    assert.equal(
      'tree_key_mismatch' in cleanClaimsReport({ schema: 1, tree_key_mismatch: v }),
      false,
      JSON.stringify(v),
    );
  assert.equal(
    'tree_key_mismatch' in cleanClaimsReport({ schema: 1 }),
    false,
    'omitted when equal',
  );
  // Unknown keys stay dropped.
  assert.equal(
    'shell' in cleanClaimsReport({ schema: 1, shell: 'id', tree_key_mismatch: true }),
    false,
  );
  // The published report carries it and the row's verdict is the Worker's own.
  const claims = cleanClaimsReport({ schema: 1, tree_key_mismatch: true });
  const row = {
    commit_sha: 'a'.repeat(40),
    tree_kind: 'tree',
    tree_hash: 'b'.repeat(64),
    tier: 'small',
    die: 'AD104',
    family: 'ada',
    verdict: 'PASS',
    started_at: null,
    ended_at: null,
  };
  const doc = buildReport({
    claims,
    row,
    runId: '123-1',
    log: null,
    offsets: null,
    logsBase: null,
  });
  assert.equal(doc.tree_key_mismatch, true);
  assert.equal(doc.verdict, 'PASS');
  const plain = buildReport({
    claims: cleanClaimsReport({ schema: 1 }),
    row,
    runId: '123-1',
    log: null,
    offsets: null,
    logsBase: null,
  });
  assert.equal('tree_key_mismatch' in plain, false);
});
