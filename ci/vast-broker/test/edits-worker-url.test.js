import test from 'node:test';
import assert from 'node:assert/strict';
import { configuration } from '../src/policy.js';
import { setup, env } from './helpers.js';
// Final review (f): a WORKER_URL with a trailing slash or a path became a wrong OIDC audience
// and a broken KF_BROKER_URL ('//prefix'). FALSIFIER: any form other than the bare https origin
// is accepted, or rent, stop-all or the cron report anything but CONFIG_VAR:WORKER_URL.
const prefix = 'worker-url-fixture-' + 'p'.repeat(32);
const bad = [
  'https://broker.example/',
  'https://broker.example/path',
  'https://broker.example/path/',
  'http://broker.example',
  'https://broker.example?x=1',
  'https://broker.example#h',
  'https://user:pass@broker.example',
  'https://Broker.example',
  'broker.example',
  '',
];
for (const value of bad)
  test('review f: WORKER_URL ' + JSON.stringify(value) + ' is CONFIG_VAR:WORKER_URL', () => {
    assert.throws(
      () => configuration({ ...env, WORKER_URL: value }),
      (e) => e.state === 'CONFIG_VAR:WORKER_URL',
    );
  });
test('review f: a missing WORKER_URL is CONFIG_VAR:WORKER_URL', () => {
  const { WORKER_URL, ...rest } = env;
  assert.throws(
    () => configuration(rest),
    (e) => e.state === 'CONFIG_VAR:WORKER_URL',
  );
});
for (const value of ['https://broker.example', 'https://broker.example:8443'])
  test('review f: the bare origin ' + value + ' is accepted', () => {
    assert.equal(configuration({ ...env, WORKER_URL: value }).audience, value);
  });
for (const [route, token] of [
  ['/rent', 'oidc'],
  ['/stop-all', 'owner'],
])
  test('review f: ' + route + ' with WORKER_URL slash answers CONFIG_VAR:WORKER_URL', async () => {
    const h = await setup({ WORKER_URL: 'https://broker.example/', BROKER_PATH_PREFIX: prefix });
    const response = await h.service.fetch(
      new Request('https://broker.example/' + prefix + route, {
        method: 'POST',
        headers: {
          Authorization: 'Bearer ' + (token === 'owner' ? env.OWNER_STOP_TOKEN : 'x.y.z'),
          'X-KF-Protocol': '5',
        },
        ...(route === '/rent' ? { body: '{}' } : {}),
      }),
    );
    assert.equal(response.status, 503);
    assert.equal((await response.json()).state, 'CONFIG_VAR:WORKER_URL');
    assert.equal(h.cloud.creates, 0);
  });
test('review f: the cron names CONFIG_VAR:WORKER_URL instead of failing silently', async () => {
  const h = await setup({ WORKER_URL: 'https://broker.example/', BROKER_PATH_PREFIX: prefix });
  const lines = [],
    log = console.log;
  console.log = (line) => lines.push(line);
  try {
    await h.service.sweep();
  } finally {
    console.log = log;
  }
  assert.deepEqual(lines.map(JSON.parse), [
    { event: 'cleanup_config_error', state: 'CONFIG_VAR:WORKER_URL' },
  ]);
});
