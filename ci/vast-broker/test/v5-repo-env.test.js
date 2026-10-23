import test from 'node:test';
import assert from 'node:assert/strict';
import { probe } from './repo-onstart.js';
for (const protocol of ['4', '5'])
  test('repo environment root-only protocol ' + protocol, () => {
    const r = probe(
      `
assert m.validate_env(env)==[]
with tempfile.TemporaryDirectory() as d:
 p=pathlib.Path(d)/'run'/'env';m.write_env_file(str(p),dict(env,VAST_API_KEY='account-value',OWNER_STOP_TOKEN='owner-value'))
 assert p.stat().st_mode&0o777==0o600 and p.parent.stat().st_mode&0o777==0o700
 text=p.read_text();assert env['KF_RUN_TOKEN'] in text and env['KF_RUNNER_NONCE'] in text
 assert 'account-value' not in text and 'owner-value' not in text
`,
      { KF_PROTOCOL: protocol },
    );
    assert.equal(r.status, 0, r.stderr);
  });
for (const protocol of ['3', '6'])
  test('repo environment refuses unsupported protocol ' + protocol, () => {
    const r = probe("assert any(p.startswith('KF_PROTOCOL:') for p in m.validate_env(env))", {
      KF_PROTOCOL: protocol,
    });
    assert.equal(r.status, 0, r.stderr);
  });
for (const field of ['KF_NONCE', 'KF_BROKER_URL'])
  test('repo environment refuses newline ' + field, () => {
    const r = probe(
      "assert any(p.startswith(sys.argv[3]+':') for p in m.validate_env(env))",
      { [field]: 'bad\ninjected' },
      [field],
    );
    assert.equal(r.status, 0, r.stderr);
  });
