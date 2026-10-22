import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, stat, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
// FALSIFIER: the real repo entrypoint stores a capability in a public file,
// persists account authority, or starts transport under an unsupported protocol.
const source = await readFile(
  new URL('../../../scripts/ci/vast_onstart.sh', import.meta.url),
  'utf8',
);
const environment = {
  KF_PROTOCOL: '5',
  KF_BROKER_URL: 'https://broker.example/' + 'p'.repeat(32),
  KF_RUN_ID: '123-1',
  KF_RUN_TOKEN: 'v1.123.1.1800030000.' + 'A'.repeat(43),
  KF_NONCE: 'b'.repeat(32),
  KF_SHA: 'a'.repeat(40),
  KF_REPO: 'owner/repo',
  KF_BOOT_DEADLINE: '1800000200',
  KF_HARD_DEADLINE: '1800000400',
  KF_SOFT_DEADLINE: '1800000300',
  KF_HEARTBEAT_SECONDS: '60',
  KF_GRACE_PASS: '60',
  KF_GRACE_FAIL: '60',
  KF_RUNNER_NONCE: 'f'.repeat(64),
};
async function run(patch = {}) {
  const dir = await mkdtemp(tmpdir() + '/v5-repo-env-');
  const script = source
    .split('cat > /usr/local/bin/kf-box-kill')[0]
    .replaceAll('/run/kf-ci', dir + '/run')
    .replaceAll('/workspace', dir + '/workspace');
  const result = spawnSync('bash', ['-s'], {
    input: script,
    encoding: 'utf8',
    env: {
      ...process.env,
      ...environment,
      VAST_API_KEY: 'test-account-value-never-persist',
      OWNER_STOP_TOKEN: 'test-owner-value-never-persist',
      ...patch,
    },
  });
  return { dir, result };
}
for (const protocol of ['4', '5'])
  test('repo environment root-only protocol ' + protocol, async () => {
    const x = await run({ KF_PROTOCOL: protocol });
    try {
      assert.equal(x.result.status, 0, x.result.stderr);
      assert.equal((await stat(x.dir + '/run')).mode & 0o777, 0o700);
      assert.equal((await stat(x.dir + '/run/env')).mode & 0o777, 0o600);
      const text = await readFile(x.dir + '/run/env', 'utf8');
      assert.ok(text.includes(environment.KF_RUN_TOKEN));
      assert.ok(text.includes(environment.KF_RUNNER_NONCE));
      assert.ok(!text.includes('test-account-value'));
      assert.ok(!text.includes('test-owner-value'));
    } finally {
      await rm(x.dir, { recursive: true });
    }
  });
for (const protocol of ['3', '6'])
  test('repo environment refuses unsupported protocol ' + protocol, async () => {
    const x = await run({ KF_PROTOCOL: protocol });
    try {
      assert.notEqual(x.result.status, 0);
      assert.match(x.result.stderr, /Unsupported box protocol/);
    } finally {
      await rm(x.dir, { recursive: true });
    }
  });
for (const field of ['KF_NONCE', 'KF_BROKER_URL'])
  test('repo environment refuses newline ' + field, async () => {
    const x = await run({ [field]: 'bad\ninjected' });
    try {
      assert.notEqual(x.result.status, 0);
      assert.match(x.result.stderr, /Invalid environment contract/);
    } finally {
      await rm(x.dir, { recursive: true });
    }
  });
