import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createGateway } from '../src/index.js';
import { setup, env, clock, jwk } from './helpers.js';
// FALSIFIER V1: a deploy without a numeric OWNER_GITHUB_ID silently cleans nothing (cron and
// stop-all give a bare UNAVAILABLE), or the diagnostic echoes the configured value.
const bad = ['REPLACE_ME', '', 'owner-42', '0042'];
const named = 'CONFIG_VAR:OWNER_GITHUB_ID';
test('round committed wrangler.toml carries the public numeric owner id', async () => {
  const toml = await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8');
  assert.match(toml, /^OWNER_GITHUB_ID = "229738317"$/m);
  assert.doesNotMatch(toml, /REPLACE_ME/);
});
test('round committed wrangler.toml vars pass owner configuration', async () => {
  const { configuration } = await import('../src/policy.js');
  const toml = await readFile(new URL('../wrangler.toml', import.meta.url), 'utf8');
  const vars = Object.fromEntries(
    [...toml.split('[vars]')[1].matchAll(/^([A-Z_]+) = (?:"([^"]*)"|'([^']*)')$/gm)].map((m) => [
      m[1],
      m[2] ?? m[3],
    ]),
  );
  const { RUN_TOKEN_SECRET, OWNER_STOP_TOKEN, VAST_API_KEY, WORKER_URL } = env;
  const cfg = configuration({
    ...vars,
    RUN_TOKEN_SECRET,
    OWNER_STOP_TOKEN,
    VAST_API_KEY,
    WORKER_URL,
  });
  assert.equal(cfg.owner, '229738317');
});
for (const value of bad) {
  test('round stop-all names the owner config error ' + JSON.stringify(value), async () => {
    const h = await setup({ OWNER_GITHUB_ID: value });
    const response = await h.call('/stop-all', { token: env.OWNER_STOP_TOKEN });
    const text = await response.text();
    assert.equal(response.status, 503);
    assert.equal(JSON.parse(text).state, named);
    if (value) assert.ok(!text.includes(value));
  });
  test('round rent refuses with the owner config error ' + JSON.stringify(value), async () => {
    const h = await setup({ OWNER_GITHUB_ID: value });
    const response = await h.call();
    assert.equal(response.status, 503);
    assert.equal((await response.json()).state, named);
    assert.equal(h.cloud.creates, 0);
  });
  test('round cron reports the owner config error ' + JSON.stringify(value), async () => {
    const h = await setup({ OWNER_GITHUB_ID: value });
    const lines = [];
    const log = console.log;
    console.log = (line) => lines.push(String(line));
    try {
      await h.service.sweep();
    } finally {
      console.log = log;
    }
    assert.deepEqual(
      lines.map((x) => JSON.parse(x)),
      [{ event: 'cleanup_config_error', state: named }],
    );
    if (value) assert.ok(!lines.join('').includes(value));
  });
}
for (const path of ['/stop-all', '/rent'])
  test('round gateway names the owner config error ' + path, async () => {
    let entries = 0;
    const gateway = createGateway({
      now: () => clock,
      fetcher: async () => Response.json({ keys: [jwk] }),
    });
    const response = await gateway.fetch(
      new Request(env.WORKER_URL + path, {
        method: 'POST',
        headers: { Authorization: 'Bearer ' + env.OWNER_STOP_TOKEN },
      }),
      {
        ...env,
        RENTING_ENABLED: 'false',
        OWNER_GITHUB_ID: 'REPLACE_ME',
        LEDGER: {
          idFromName: () => 1,
          get: () => ({ fetch: async () => (entries++, new Response('admitted')) }),
        },
      },
    );
    const text = await response.text();
    assert.equal(response.status, 503);
    assert.equal(JSON.parse(text).state, named);
    assert.ok(!text.includes('REPLACE_ME'));
    assert.equal(entries, 0);
  });
