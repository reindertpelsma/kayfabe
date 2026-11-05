import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, env } from './helpers.js';
import { BOOLEAN_VARS, configuration, teardownConfiguration } from '../src/policy.js';
import { ALLOWED, BOOLEAN_VARS as DEPLOY_BOOLEANS, parseVars } from '../deploy.mjs';
// Review of PR #269, R3 (2026-11-05). FALSIFIERS: deploy.mjs accepts a boolean var that is not
// exactly 'true' or 'false'; the Worker accepts one; a malformed policy var stops the cron from
// tearing down an open rental past its deadline; the cron starts or expires anything while the
// configuration is malformed; an identity error is downgraded to the defaults.

const BAD = ['yes', 'TRUE', 'False', '1', '0', 'on', 'true,'];
const OWNER = 'OWNER_GITHUB_ID=' + env.OWNER_GITHUB_ID + '\n';

test('R3: deploy.mjs and the Worker name the same boolean vars, all deployable', () => {
  assert.deepEqual([...DEPLOY_BOOLEANS], [...BOOLEAN_VARS]);
  for (const name of BOOLEAN_VARS) assert.ok(ALLOWED.has(name), name);
});

for (const name of BOOLEAN_VARS) {
  test('R3: deploy.mjs accepts only true|false for ' + name, () => {
    for (const value of ['true', 'false'])
      assert.equal(parseVars(OWNER + name + '=' + value + '\n').get(name), value);
    for (const value of BAD)
      assert.throws(
        () => parseVars(OWNER + name + '=' + value + '\n'),
        (e) => e.message === name + ' must be true or false',
        value,
      );
  });
  test('R3: configuration() refuses a malformed ' + name + ' by name', () => {
    for (const value of ['true', 'false', undefined])
      assert.doesNotThrow(() => configuration({ ...env, [name]: value }), String(value));
    for (const value of [...BAD, ''])
      assert.throws(
        () => configuration({ ...env, [name]: value }),
        (e) => e.state === 'CONFIG_VAR:' + name,
        value,
      );
  });
}

test('R3: the teardown configuration drops policy vars, never identity', () => {
  const cfg = teardownConfiguration({ ...env, RELEASE_ENABLED: 'yes', MAX_PRICE_USD_HOUR: 'x' });
  assert.equal(cfg.owner, env.OWNER_GITHUB_ID);
  assert.equal(cfg.releaseEnabled, false);
  assert.throws(() => teardownConfiguration({ ...env, OWNER_GITHUB_ID: 'x' }));
});

async function capture(fn) {
  const lines = [],
    log = console.log;
  console.log = (line) => lines.push(String(line));
  try {
    await fn();
  } finally {
    console.log = log;
  }
  return lines.map((l) => {
    try {
      return JSON.parse(l);
    } catch {
      return { raw: l };
    }
  });
}

for (const [name, value, state] of [
  ['RELEASE_ENABLED', 'yes', 'CONFIG_VAR:RELEASE_ENABLED'],
  ['RENTING_ENABLED', 'TRUE', 'CONFIG_VAR:RENTING_ENABLED'],
  ['ALLOW_UNKNOWN_COUNTRY', 'on', 'CONFIG_VAR:ALLOW_UNKNOWN_COUNTRY'],
  ['TREE_KEY_CARRY_OVER', '1', 'CONFIG_VAR:TREE_KEY_CARRY_OVER'],
  ['MAX_PRICE_USD_HOUR', 'abc', 'CONFIG_UNNAMED'],
])
  test('R3: a malformed ' + name + ' refuses rentals but the cron still tears down', async () => {
    const h = await setup();
    assert.equal((await h.call()).status, 200);
    const record = await h.storage.get('record:123-1');
    // An old ended record that a normal sweep would expire (retention passed).
    await h.storage.put('ended:999-1', h.time.value - 30 * 86400000);
    await h.storage.put('record:999-1', { key: '999-1', ended: 1, id: null });
    h.bindings[name] = value;
    const refused = await h.call();
    assert.equal(refused.status, 503);
    assert.equal((await refused.json()).state, state === 'CONFIG_UNNAMED' ? 'UNAVAILABLE' : state);
    assert.equal(h.cloud.creates, 1);
    h.time.value = record.hardDeadline + 1;
    const lines = await capture(() => h.service.sweep());
    assert.deepEqual(lines[0], { event: 'cleanup_config_error', state, teardown: 'defaults' });
    // Nothing after the teardown runs: no batch tick (which would refuse), no expiry, no store.
    assert.equal(
      lines.filter((l) => /^cleanup_/.test(l.event ?? '')).length,
      1,
      JSON.stringify(lines),
    );
    const after = await h.storage.get('record:123-1');
    assert.notEqual(after.ended, null, 'the box past its hard deadline was closed');
    assert.ok(h.cloud.deletes.includes(String(record.id)), 'destroyed by exact id');
    assert.equal(h.cloud.creates, 1, 'nothing rented');
    assert.notEqual(await h.storage.get('ended:999-1'), undefined, 'no expiry on defaults');
  });
