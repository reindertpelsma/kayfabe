import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { probe, repoSource, helperProbe, boxEnv } from './repo-onstart.js';
for (const prefix of ['', 'hold-prefix-' + 'h'.repeat(32)])
  test('V5 generated hold helper is scoped HTTPS ' + prefix.length, async () => {
    const env = boxEnv({ KF_BROKER_URL: 'https://broker.example' + (prefix ? '/' + prefix : '') });
    const r = await helperProbe('kf-box-hold', [], env);
    assert.equal(r.status, 0, r.stderr);
    assert.equal(r.args.at(-1), env.KF_BROKER_URL + '/hold/123-1');
    assert.ok(r.stdin.includes('Authorization: Bearer ' + env.KF_RUN_TOKEN));
    assert.ok(!r.args.join(' ').includes(env.KF_RUN_TOKEN));
    assert.ok(r.args.includes('=https'));
    assert.ok(!r.args.includes('-L'));
  });
for (const ssh of [false, 'ipv4', 'ipv6'])
  test('V5 profile hold hook runs only on SSH login ' + ssh, async () => {
    const dir = await mkdtemp(tmpdir() + '/v5-hook-');
    try {
      await writeFile(dir + '/hold', `#!/bin/sh\nprintf called >> '${dir}/called'\n`, {
        mode: 0o700,
      });
      const r = spawnSync('bash', ['--noprofile', '--norc', '-ic', repoSource('kf-ci-hold.sh')], {
        encoding: 'utf8',
        env: {
          ...process.env,
          KF_CI_HOLD_DONE: '',
          KF_BOX_HOLD_BIN: dir + '/hold',
          SSH_CONNECTION: ssh ? '192.0.2.1 1 192.0.2.2 2' : '',
        },
      });
      assert.equal(r.status, 0, r.stderr);
      let text = '';
      for (let i = 0; i < (ssh ? 100 : 1); i++) {
        try {
          text = await readFile(dir + '/called', 'utf8');
          break;
        } catch {}
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
      assert.equal(text, ssh ? 'called' : '');
    } finally {
      await rm(dir, { recursive: true });
    }
  });
for (const exhausted of ['attempts', 'bytes'])
  test('V5 empty-log heartbeat survives sender cap ' + exhausted, () => {
    const r = probe(
      `
sender.requests=limits.max_requests if sys.argv[3]=='attempts' else 0
sender.bytes=limits.max_bytes if sys.argv[3]=='bytes' else 0
sender.feed(b'bounded output');clock.value+=sender.heartbeat+1;sender.pump(True)
assert len(transport.calls)==1 and transport.calls[0]['body']['kind']=='log' and transport.calls[0]['body']['text']==''
`,
      {},
      [exhausted],
    );
    assert.equal(r.status, 0, r.stderr);
  });
for (const exhausted of ['attempts', 'bytes'])
  test('V5 sender busy output cannot starve heartbeat ' + exhausted, () => {
    const r = probe(
      `
sender.requests=limits.max_requests if sys.argv[3]=='attempts' else 0
sender.bytes=limits.max_bytes if sys.argv[3]=='bytes' else 0
for i in range(100):
 sender.feed(b'payload output\\n'*500);clock.value+=10;sender.pump(True)
assert any(c['body']['kind']=='log' and c['body'].get('text')=='' for c in transport.calls)
`,
      {},
      [exhausted],
    );
    assert.equal(r.status, 0, r.stderr);
  });
for (const prefix of ['', 'payload-prefix-' + 'x'.repeat(32)])
  test('V5 onstart invokes exact approved payload ' + prefix.length, () => {
    const s = repoSource('vast_onstart.sh');
    assert.ok(s.includes('payload=scripts/ci/vast_payload.sh'));
    assert.ok(s.includes('bash "$payload" "$KF_SHA" "$KF_NONCE"'));
    assert.ok(s.includes('KF_SOFT_DEADLINE_MS='));
    assert.ok(!s.includes('vast_box_main.sh'));
  });
