import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
async function client(
  state,
  verdict = 'UNAVAILABLE',
  code = 200,
  pending = false,
  forcedResult,
  verified,
  policyResult = 'success',
  policyRun = '1',
) {
  const p = spawnSync(
    'python3',
    [
      '-I',
      new URL('round3_main_fixture.py', import.meta.url).pathname,
      JSON.stringify({ state, verdict, code, pending }),
    ],
    { encoding: 'utf8', timeout: 5000 },
  );
  assert.equal(p.status, 0, p.stderr);
  const controller = JSON.parse(p.stdout);
  const directory = await mkdtemp(join(tmpdir(), 'v3-client-'));
  try {
    const yaml = await readFile(
      new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
      'utf8',
    );
    const script = (id) =>
      yaml
        .split('  ' + id + ':\n')[1]
        .split(/\n  [\w-]+:\n/)[0]
        .split('        run: |\n')[1]
        .split('\n')
        .map((x) => (x.startsWith('          ') ? x.slice(10) : x))
        .join('\n');
    await writeFile(
      join(directory, 'curl'),
      `#!/usr/bin/env python3
import json,os,sys
args=sys.argv[1:];url=args[-1];assert url=='https://api.github.com/repos/reindertpelsma/kayfabe/statuses/'+'a'*40
assert not any('X-GH-Token' in x for x in args)
body=json.load(open(args[args.index('--data-binary')+1][1:]))
with open(os.environ['STATUS_LOG'],'a') as f:f.write(json.dumps(body)+'\\n')
`,
      { mode: 0o700 },
    );
    const fields = Object.fromEntries(
      controller.outputs
        .trim()
        .split('\n')
        .map((x) => x.split('=')),
    );
    const env = {
      ...process.env,
      PATH: directory + ':' + process.env.PATH,
      GH_TOKEN: 'mock-status-token',
      TARGET_SHA: 'a'.repeat(40),
      GITHUB_REPOSITORY: 'reindertpelsma/kayfabe',
      GITHUB_SERVER_URL: 'https://github.com',
      GITHUB_RUN_ID: '123',
      STATUS_LOG: join(directory, 'statuses'),
      PENDING_RESULT: 'success',
      POLICY_RESULT: policyResult,
      POLICY_RUN: policyRun,
      // The action-free policy-check agrees on the decision; each job result is checked alone.
      POLICY_CHECK_RESULT: 'success',
      POLICY_CHECK_RUN: policyRun,
      GPU_RESULT: forcedResult ?? (controller.status === 0 ? 'success' : 'failure'),
      BROKER_STATE: fields.state,
      BROKER_VERDICT: fields.verdict,
      BROKER_VERIFIED: verified ?? fields.verified,
    };
    const initial = spawnSync(
      'bash',
      ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script('pending')],
      { env, encoding: 'utf8', timeout: 5000 },
    );
    assert.equal(initial.status, 0, initial.stderr);
    const final = spawnSync(
      'bash',
      ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script('report')],
      { env, encoding: 'utf8', timeout: 5000 },
    );
    return {
      ...controller,
      status: final.status,
      statuses: (await readFile(join(directory, 'statuses'), 'utf8'))
        .trim()
        .split('\n')
        .map(JSON.parse),
      stops: controller.calls.filter((x) => x.url.includes('/stop/')).length,
    };
  } finally {
    await rm(directory, { recursive: true });
  }
}
test('workflow posts success on exact head after runner independently scrubs hostile log', async () => {
  const r = await client('GRACE', 'PASS');
  assert.equal(r.status, 0);
  assert.deepEqual(
    r.statuses.map((x) => x.state),
    ['pending', 'success'],
  );
  assert.match(r.stdout, /::stop-commands::[0-9a-f]+\nUNTRUSTED/);
  assert.doesNotMatch(r.summary, /::error::/);
  assert.match(r.outputs, /^state=GRACE\nverdict=PASS\nverified=1\n$/);
});
test('workflow posts failure when structured box verdict fails', async () => {
  const r = await client('GRACE', 'FAIL');
  assert.equal(r.status, 1);
  assert.equal(r.statuses.at(-1).state, 'failure');
});
for (const [state, code] of [
  ['UNAVAILABLE_BUDGET', 409],
  ['NEEDS_APPROVAL', 403],
])
  test('workflow exposes failing refusal ' + state, async () => {
    const r = await client(state, 'UNAVAILABLE', code);
    assert.equal(r.status, 1);
    assert.match(r.statuses.at(-1).description, new RegExp(state));
  });
for (const pending of [false, true])
  test('terminal unavailable completes without further polls ' + pending, async () => {
    const r = await client('GRACE', 'UNAVAILABLE', 200, pending);
    assert.equal(r.status, 1);
    assert.equal(r.calls.filter((x) => /\/(rent|status\/)/.test(x.url)).length, pending ? 2 : 1);
  });
test('NOT_NEEDED is explicit successful status bound to nonce', async () => {
  const r = await client('NOT_NEEDED', 'PASS');
  assert.equal(r.status, 0);
  assert.equal(r.statuses.at(-1).state, 'success');
  assert.match(r.statuses.at(-1).description, /NOT_NEEDED/);
});
for (const dependency of ['skipped', 'cancelled'])
  test('final status rejects ' + dependency + ' even with verified PASS', async () => {
    const r = await client('GRACE', 'PASS', 200, false, dependency);
    assert.equal(r.status, 1);
    assert.notEqual(r.statuses.at(-1).state, 'success');
  });
for (const verdict of ['FAIL', 'UNAVAILABLE'])
  test('terminal ' + verdict + ' preserves inspection grace', async () => {
    const r = await client('GRACE', verdict);
    assert.equal(r.status, 1);
    assert.equal(r.stops, 0);
  });
for (const verdict of ['PASS', 'FAIL'])
  test('cursor log appears once live and once in final bounded group ' + verdict, async () => {
    const r = await client('GRACE', verdict, 200, true);
    assert.equal((r.stdout.match(/\[box\] : error: hostile/g) || []).length, 2);
    assert.equal(r.stops, 0);
  });
for (const verified of ['0', 'true'])
  test('final status rejects unverified PASS ' + verified, async () => {
    const r = await client('GRACE', 'PASS', 200, false, undefined, verified);
    assert.equal(r.status, 1);
    assert.notEqual(r.statuses.at(-1).state, 'success');
  });

for (const verdict of ['PASS', 'UNAVAILABLE'])
  test('trusted path policy explicitly succeeds without a GPU job ' + verdict, async () => {
    const r = await client('GRACE', verdict, 200, false, 'skipped', '0', 'success', '0');
    assert.equal(r.status, 0);
    assert.equal(r.statuses.at(-1).state, 'success');
    assert.match(r.statuses.at(-1).description, /NOT_NEEDED/);
  });
for (const result of ['failure', 'skipped'])
  test('failed path policy never permits a skipped GPU job ' + result, async () => {
    const r = await client('GRACE', 'PASS', 200, false, 'skipped', '1', result, '0');
    assert.equal(r.status, 1);
    assert.notEqual(r.statuses.at(-1).state, 'success');
  });
