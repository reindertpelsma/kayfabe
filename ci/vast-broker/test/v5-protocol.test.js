import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { setup, nonce, sha } from './helpers.js';
const names = {
  provision: 1,
  kf_tests: 100,
  gates: 9,
  kf3_build: 1,
  raw_suite: 30,
  thin_suite: 30,
  birth_census: 1,
  cuda_host: 4,
  cuda_fat: 4,
};
const suites = () =>
  Object.entries(names).map(([name, pass]) => ({ name, pass, fail: 0, skip: 0 }));
const hardware = { gpu_name: 'RTX_4070', compute_cap: 890, driver_version: '580.159.04' };
// FALSIFIER: exit/log text implies PASS, a malformed suite table passes, or the
// protected runner accepts Worker metadata without validating every field.
for (const verdict of ['PASS', 'FAIL'])
  test('V5 structured suites reach status ' + verdict, async () => {
    const h = await setup();
    await h.call();
    const table = suites();
    if (verdict === 'FAIL') {
      table[0].fail = 1;
      table[0].pass = 0;
    }
    const r = await h.report({
      seq: 1,
      kind: 'result',
      result: { nonce, verdict, suites: table, hardware },
    });
    assert.equal(r.status, 200);
    const d = await (await h.call('/status/123-1')).json();
    assert.equal(d.protocol_version, 5);
    assert.equal(d.worker_version, 'v5');
    assert.deepEqual(d.suites, table);
    assert.equal(d.driver_version, hardware.driver_version);
    assert.equal(d.die, 'AD104');
    assert.equal(d.selection_reason, 'DEFAULT_ORDER');
    assert.equal(d.state, 'GRACE');
  });
for (const kind of [
  'missing',
  'empty',
  'zero-pass',
  'skip',
  'fail',
  'duplicate',
  'unknown',
  'extra',
  'negative',
  'bad-driver',
  'bad-gpu',
  'bad-compute',
])
  test('V5 incomplete result cannot PASS ' + kind, async () => {
    const h = await setup();
    await h.call();
    const result = { nonce, verdict: 'PASS', suites: suites(), hardware: { ...hardware } };
    if (kind === 'missing') delete result.suites;
    if (kind === 'empty') result.suites = [];
    if (kind === 'zero-pass') result.suites[0].pass = 0;
    if (kind === 'skip') result.suites[0].skip = 1;
    if (kind === 'fail') result.suites[0].fail = 1;
    if (kind === 'duplicate') result.suites.push(result.suites[0]);
    if (kind === 'unknown') result.suites[0].name = '::error::';
    if (kind === 'extra') result.suites[0].command = '::error::';
    if (kind === 'negative') result.suites[0].pass = -1;
    if (kind === 'bad-driver') result.hardware.driver_version = '::error::';
    if (kind === 'bad-gpu') result.hardware.gpu_name = '::error::';
    if (kind === 'bad-compute') result.hardware.compute_cap = '890';
    assert.equal((await h.report({ seq: 1, kind: 'result', result })).status, 400);
    assert.equal((await h.storage.get('record:123-1')).resultAt, undefined);
  });
for (const phase of ['GRACE', 'NONE'])
  test('V5 runner validates terminal suite metadata ' + phase, () => {
    const r = spawnSync(
      'python3',
      ['-I', new URL('v5_protocol_fixture.py', import.meta.url).pathname, phase],
      { encoding: 'utf8', timeout: 10000 },
    );
    assert.equal(r.status, 0, r.stderr + r.stdout);
  });
