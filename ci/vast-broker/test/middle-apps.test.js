// Falsifier: untrusted request fields override signed tier/owner policy, or an app request is lost.
// Local signed OIDC and provider mocks only: no rental, deployment or network.
import test from 'node:test';
import assert from 'node:assert/strict';
import { setup, rentBody, sha, env, nonce, sampleResult } from './helpers.js';
import { MIDDLE_APP_COUNTS } from '../src/tier.js';
import { readFileSync } from 'node:fs';
import { configuration } from '../src/policy.js';

const configured = (extra = {}) => setup({ MIDDLE_APPS_ENABLED: undefined, ...extra });
const requested = () => rentBody(sha, { linuxApps: true, winApps: 'mid' });
const created = (h) => h.cloud.calls.find((c) => /\/api\/v0\/asks\/\d+\/$/.test(c.url))?.body.env;

test('middle policy defaults on for signed PR claims and requests both apps', async () => {
  const h = await configured();
  const p = await h.call('/availability?profile=check&apps=1', { method: 'GET' });
  assert.equal(p.status, 200);
  assert.deepEqual((await p.json()).apps_policy, { run: '123-1', tier: 'middle', enabled: true });
  const response = await h.call('/rent', { body: requested() });
  assert.equal(response.status, 200, await response.clone().text());
  assert.equal(created(h).KF_LINUX_APPS, '1');
  assert.equal(created(h).KF_WINAPPS, 'mid');
});
test('a caller cannot disable Linux apps on a signed middle run', async () => {
  const h = await configured();
  const response = await h.call('/rent', { body: { ...requested(), linuxApps: false } });
  assert.equal(response.status, 400);
  assert.equal(h.cloud.creates, 0);
});
test('a caller cannot disable Windows apps on a signed middle run', async () => {
  const h = await configured();
  const response = await h.call('/rent', { body: rentBody(sha, { linuxApps: true }) });
  assert.equal(response.status, 400);
  assert.equal(h.cloud.creates, 0);
});
test('a middle caller cannot substitute another Windows profile', async () => {
  const h = await configured();
  assert.equal((await h.call('/rent', { body: { ...requested(), winApps: 'small' } })).status, 400);
  assert.equal(h.cloud.creates, 0);
});
test('a caller cannot substitute full Windows apps either', async () => {
  const h = await configured();
  assert.equal((await h.call('/rent', { body: { ...requested(), winApps: 'full' } })).status, 400);
  assert.equal(h.cloud.creates, 0);
});
test('request tier field cannot downgrade a signed PR tier', async () => {
  const h = await configured();
  assert.equal((await h.call('/rent', { body: { ...requested(), tier: 'small' } })).status, 400);
  assert.equal(h.cloud.creates, 0);
});
test('request kill-switch field cannot override owner policy', async () => {
  const h = await configured();
  assert.equal(
    (await h.call('/rent', { body: { ...requested(), MIDDLE_APPS_ENABLED: false } })).status,
    400,
  );
  assert.equal(h.cloud.creates, 0);
});
test('push policy never enables either app stage', async () => {
  const h = await configured();
  h.cloud.run.event = 'push';
  h.cloud.run.head_branch = 'master';
  const claims = { event_name: 'push', ref: 'refs/heads/master' };
  const p = await h.call('/availability?profile=check&apps=1', { method: 'GET', claims });
  assert.deepEqual((await p.json()).apps_policy, { run: '123-1', tier: 'small', enabled: false });
  assert.equal((await h.call('/rent', { claims, body: rentBody(sha) })).status, 200);
  assert.equal(created(h).KF_LINUX_APPS, undefined);
  assert.equal(created(h).KF_WINAPPS, undefined);
});
test('push cannot enable apps through an explicit request', async () => {
  const h = await configured();
  h.cloud.run.event = 'push';
  h.cloud.run.head_branch = 'master';
  const claims = { event_name: 'push', ref: 'refs/heads/master' };
  assert.equal((await h.call('/rent', { claims, body: requested() })).status, 400);
  assert.equal(h.cloud.creates, 0);
});
test('owner false switches off both stages without a source edit', async () => {
  const h = await configured({ MIDDLE_APPS_ENABLED: 'false' });
  const p = await h.call('/availability?profile=check&apps=1', { method: 'GET' });
  assert.equal((await p.json()).apps_policy.enabled, false);
  assert.equal((await h.call('/rent', { body: rentBody(sha) })).status, 200);
  assert.equal(created(h).KF_LINUX_APPS, undefined);
  assert.equal(created(h).KF_WINAPPS, undefined);
});
test('owner false cannot be overridden by a middle request', async () => {
  const h = await configured({ MIDDLE_APPS_ENABLED: 'false' });
  assert.equal((await h.call('/rent', { body: requested() })).status, 400);
  assert.equal(h.cloud.creates, 0);
});
test('invalid owner switch is refused rather than silently disabling apps', () => {
  for (const value of ['0', 'no', '', 'False'])
    assert.throws(
      () => configuration({ ...env, MIDDLE_APPS_ENABLED: value }),
      (e) => e.state === 'CONFIG_VAR:MIDDLE_APPS_ENABLED',
    );
});

test('middle preferences do not drop either real-app request', async () => {
  const h = await configured();
  const response = await h.call('/rent', { body: { ...requested(), preferences: ['AD104'] } });
  assert.equal(response.status, 200, await response.clone().text());
  assert.equal(created(h).KF_LINUX_APPS, '1');
  assert.equal(created(h).KF_WINAPPS, 'mid');
});
test('malformed owner switch refuses authenticated policy reads before rental', async () => {
  const h = await configured({ MIDDLE_APPS_ENABLED: '0' });
  const response = await h.call('/availability?profile=check&apps=1', { method: 'GET' });
  assert.equal(response.status, 503);
  assert.equal(h.cloud.creates, 0);
});

for (const missing of ['apps', 'windows_apps']) {
  test('Worker never stores PASS when requested ' + missing + ' evidence is missing', async () => {
    const h = await configured();
    assert.equal((await h.call('/rent', { body: requested() })).status, 200);
    const result = { nonce, verdict: 'PASS', ...sampleResult() };
    result.suites.push(
      ...Object.entries({ apps: 63, windows_apps: 10 })
        .filter(([name]) => name !== missing)
        .map(([name, pass]) => ({ name, pass, fail: 0, skip: 0 })),
    );
    assert.equal((await h.report({ seq: 1, kind: 'result', result })).status, 200);
    const stored = (await h.storage.get('record:123-1')).result;
    assert.equal(stored.verdict, 'FAIL');
    assert.ok(
      stored.suites.some((row) => row.name === missing + '_retry_operator' && row.fail === 1),
    );
  });
}
for (const failed of ['apps', 'windows_apps']) {
  test('Worker preserves an attributed guest failure in ' + failed, async () => {
    const h = await configured();
    assert.equal((await h.call('/rent', { body: requested() })).status, 200);
    const result = { nonce, verdict: 'FAIL', ...sampleResult() };
    result.suites.push({ name: failed, pass: 0, fail: 1, skip: 0 });
    assert.equal((await h.report({ seq: 1, kind: 'result', result })).status, 200);
    const stored = (await h.storage.get('record:123-1')).result;
    assert.equal(stored.verdict, 'FAIL');
    assert.ok(!stored.suites.some((row) => row.name.endsWith('_retry_operator')));
  });
}

test('Windows mid count is pinned and excludes manual/unsupported apps', () => {
  const p = JSON.parse(
    readFileSync(
      new URL('../../../scripts/bench/windows/appmatrix/gate_profiles.json', import.meta.url),
    ),
  );
  assert.equal(p.profiles.mid.apps.length, MIDDLE_APP_COUNTS.windows_apps);
  assert.equal(p.profiles.mid.budget_s, 3600);
  for (const id of p.profiles.mid.apps) {
    assert.ok(!p.manual.apps.includes(id));
    assert.ok(!Object.hasOwn(p.known_unsupported, id));
  }
});
test('Linux middle count is pinned to its supported gate list', () => {
  const text = readFileSync(
    new URL('../../../scripts/apps/gate_list_middle.txt', import.meta.url),
    'utf8',
  );
  const ids = text.split('\n').filter((line) => line && !line.startsWith('#'));
  const p = JSON.parse(
    readFileSync(new URL('../../../scripts/apps/known_unsupported.json', import.meta.url)),
  );
  assert.equal(ids.length, MIDDLE_APP_COUNTS.apps);
  for (const id of ids) assert.ok(!Object.hasOwn(p.known_unsupported, id));
});

test('Linux opt-in cannot be omitted while requesting the middle Windows profile', async () => {
  const h = await configured();
  assert.equal((await h.call('/rent', { body: rentBody(sha, { winApps: 'mid' }) })).status, 400);
  assert.equal(h.cloud.creates, 0);
});
for (const heartbeat of [false, true]) {
  test('complete real-app evidence remains PASS, progress=' + heartbeat, async () => {
    const h = await configured();
    assert.equal((await h.call('/rent', { body: requested() })).status, 200);
    if (heartbeat)
      assert.equal((await h.report({ seq: 1, kind: 'progress', text: 'alive' })).status, 200);
    const result = { nonce, verdict: 'PASS', ...sampleResult() };
    result.suites.push(
      ...Object.entries(MIDDLE_APP_COUNTS).map(([name, pass]) => ({
        name,
        pass,
        fail: 0,
        skip: 0,
      })),
    );
    assert.equal((await h.report({ seq: heartbeat ? 2 : 1, kind: 'result', result })).status, 200);
    assert.equal((await h.storage.get('record:123-1')).result.verdict, 'PASS');
  });
}
for (const missing of ['apps', 'windows_apps']) {
  test('operator narrowing remains bounded and unique for missing ' + missing, async () => {
    const h = await configured();
    assert.equal((await h.call('/rent', { body: requested() })).status, 200);
    const result = { nonce, verdict: 'PASS', ...sampleResult() };
    result.suites.push({ name: missing + '_retry_operator', pass: 1, fail: 0, skip: 0 });
    const other = missing === 'apps' ? 'windows_apps' : 'apps';
    result.suites.push({ name: other, pass: MIDDLE_APP_COUNTS[other], fail: 0, skip: 0 });
    while (result.suites.length < 20)
      result.suites.push({ name: 'extra_' + result.suites.length, pass: 1, fail: 0, skip: 0 });
    assert.equal((await h.report({ seq: 1, kind: 'result', result })).status, 200);
    const stored = (await h.storage.get('record:123-1')).result;
    assert.equal(stored.verdict, 'FAIL');
    assert.equal(stored.suites.length, 20);
    assert.equal(new Set(stored.suites.map((row) => row.name)).size, 20);
  });
}
for (const wrong of ['apps', 'windows_apps']) {
  test('Worker refuses a reduced real-app count for ' + wrong, async () => {
    const h = await configured();
    assert.equal((await h.call('/rent', { body: requested() })).status, 200);
    const result = { nonce, verdict: 'PASS', ...sampleResult() };
    result.suites.push(
      ...Object.entries(MIDDLE_APP_COUNTS).map(([name, count]) => ({
        name,
        pass: count - (name === wrong ? 1 : 0),
        fail: 0,
        skip: 0,
      })),
    );
    assert.equal((await h.report({ seq: 1, kind: 'result', result })).status, 200);
    const stored = (await h.storage.get('record:123-1')).result;
    assert.equal(stored.verdict, 'FAIL');
    assert.ok(
      stored.suites.some((row) => row.name === wrong + '_retry_operator' && row.fail === 1),
    );
  });
}

test('mid Windows app profile contains every small app without a union override', () => {
  const p = JSON.parse(
    readFileSync(
      new URL('../../../scripts/bench/windows/appmatrix/gate_profiles.json', import.meta.url),
    ),
  );
  assert.ok(p.profiles.small.apps.every((id) => p.profiles.mid.apps.includes(id)));
});
test('large tier contract inherits middle and adds SPEC plus reserved LLM', async () => {
  const { TIER_CONTENTS } = await import('../src/tier.js');
  assert.equal(TIER_CONTENTS.small.base, 'check');
  assert.equal(TIER_CONTENTS.middle.inherits, 'small');
  assert.deepEqual(TIER_CONTENTS.middle.appProfiles, { linux: 'middle', windows: 'mid' });
  assert.equal(TIER_CONTENTS.large.inherits, 'middle');
  assert.deepEqual(TIER_CONTENTS.large.additionalStages, ['specviewperf', 'llm_parity']);
  assert.equal(TIER_CONTENTS.large.state, 'DESIGN_ONLY');
});
test('the LLM slot is reserved in the release catalogue and cannot be enabled', async () => {
  const { DEFAULTS } = await import('../src/defaults.js');
  assert.equal(DEFAULTS.stageSlots.llm_parity?.state, 'DESIGN_ONLY');
  assert.throws(
    () => configuration({ ...env, RELEASE_STAGES: '["llm_parity"]' }),
    (e) => e.state === 'CONFIG_VAR:RELEASE_STAGES',
  );
});
test('SPEC plus LLM cannot enable an unimplemented release stage', () => {
  assert.throws(
    () => configuration({ ...env, RELEASE_STAGES: '["specviewperf","llm_parity"]' }),
    (e) => e.state === 'CONFIG_VAR:RELEASE_STAGES',
  );
});

test('large retains the complete inherited probe profiles before adding workloads', async () => {
  const { TIER_CONTENTS } = await import('../src/tier.js');
  function profiles(name) {
    const tier = TIER_CONTENTS[name];
    return { ...(tier.inherits ? profiles(tier.inherits) : {}), ...(tier.appProfiles ?? {}) };
  }
  assert.deepEqual(profiles('small'), {});
  assert.deepEqual(profiles('middle'), { linux: 'middle', windows: 'mid' });
  assert.deepEqual(profiles('large'), profiles('middle'));
  assert.ok(TIER_CONTENTS.large.additionalStages.includes('specviewperf'));
  assert.ok(TIER_CONTENTS.large.additionalStages.includes('llm_parity'));
});
