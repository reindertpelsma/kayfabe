import test from 'node:test';
import assert from 'node:assert/strict';
import { onstart } from '../src/onstart.js';
import { OidcVerifier } from '../src/oidc.js';
import { setup, env, sha, nonce, jwt, jwk, clock } from './helpers.js';
// FALSIFIER: bootstrap contains capability/transport logic, create omits its
// env contract, random kids amplify JWKS, or invalid claims reach GitHub.
for (const path of ['/rent', '/availability?profile=check'])
  test('renting requires pre-OIDC prefix ' + path, async () => {
    const h = await setup({ BROKER_PATH_PREFIX: '' });
    const r = await h.call(path, { method: path === '/rent' ? 'POST' : 'GET' });
    assert.equal(r.status, 503);
    assert.equal(h.cloud.calls.length, 0);
  });
for (const repo of ['reindertpelsma/kayfabe', 'example/project'])
  test('tiny onstart golden template ' + repo, () => {
    const s = onstart(sha, repo);
    // The sparse list itself is pinned by onstart-sparse.test.js; here the fixed frame around it.
    const list = /<<'KF_SPARSE'\n([\s\S]+)\nKF_SPARSE\n/.exec(s)[1];
    assert.equal(
      s,
      `#!/bin/bash\nset -eu\ngit clone --filter=blob:none --no-checkout https://github.com/${repo}.git /workspace/kf-ci-src\ncd /workspace/kf-ci-src\ngit config core.sparseCheckout true\ngit config core.sparseCheckoutCone false\nmkdir -p .git/info\ncat >.git/info/sparse-checkout <<'KF_SPARSE'\n${list}\nKF_SPARSE\ngit fetch --depth=1 origin ${sha}\ngit checkout --detach ${sha}\n[ "$(git rev-parse HEAD)" = '${sha}' ]\nexec bash scripts/ci/vast_onstart.sh\n`,
    );
    assert.ok(!s.includes('v1.'));
    assert.ok(!s.includes('KF_REPORT'));
  });
for (const field of ['KF_RUN_TOKEN', 'KF_SHA'])
  test('create carries environment contract ' + field, async () => {
    const h = await setup();
    await h.call();
    const b = h.cloud.calls.find((c) => c.url.endsWith('/asks/5/')).body;
    assert.equal(typeof b.env, 'object');
    assert.ok(typeof b.env[field] === 'string' && b.env[field].length > 0);
    assert.ok(!b.onstart.includes('v1.'));
    assert.ok(!JSON.stringify(b.env).includes(h.bindings.VAST_API_KEY));
    assert.ok(!JSON.stringify(b.env).includes(h.bindings.OWNER_STOP_TOKEN));
  });
for (const kind of ['kid', 'signature'])
  test('JWKS cooldown bounds 1000 unauthorized tokens ' + kind, async () => {
    let calls = 0;
    const v = new OidcVerifier(
      async () => {
        calls++;
        return Response.json({ keys: [jwk] });
      },
      () => clock,
    );
    await v.verify(await jwt(), env.WORKER_URL);
    const good = await jwt();
    for (let i = 0; i < 1000; i++) {
      let token = good;
      if (kind === 'kid') {
        const p = good.split('.');
        p[0] = Buffer.from(JSON.stringify({ alg: 'RS256', kid: 'random-' + i })).toString(
          'base64url',
        );
        token = p.join('.');
      } else token = token.slice(0, -5) + 'AAAAA';
      await assert.rejects(v.verify(token, env.WORKER_URL));
    }
    assert.equal(calls, 1);
  });
for (const patch of [{ repository_owner_id: '43' }, { ref: 'refs/heads/evil' }, { sha: 'bad' }])
  test('locally ineligible signed claims never reach GitHub ' + JSON.stringify(patch), async () => {
    const h = await setup();
    assert.notEqual((await h.call('/rent', { claims: patch })).status, 200);
    assert.equal(
      h.cloud.calls.filter((c) => new URL(c.url).hostname === 'api.github.com').length,
      0,
    );
  });
