import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
// FALSIFIER V7: a mutable action tag runs in the trusted workflow, or the job that runs it can
// turn the required gpu-vast status green on its own (NOT_NEEDED) without a Worker verdict.
const read = (name) =>
  readFile(new URL('../../../.github/workflows/' + name, import.meta.url), 'utf8');
const SETUP_NODE = 'actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020';
const job = (yaml, id) => yaml.split('  ' + id + ':\n')[1].split(/\n  [\w-]+:\n/)[0];
for (const name of ['gpu-vast.yml', 'gpu-vast-sweep.yml'])
  test('round V7 every action is pinned by full commit SHA ' + name, async () => {
    const uses = [...(await read(name)).matchAll(/^\s*-?\s*uses:\s*(\S+)(.*)$/gm)];
    for (const [, ref] of uses) assert.match(ref, /^[\w.-]+\/[\w./-]+@[0-9a-f]{40}$/, ref);
  });
test('round V7 setup-node is the reviewed v4 commit', async () => {
  const yaml = await read('gpu-vast.yml');
  assert.ok(yaml.includes('uses: ' + SETUP_NODE + ' # v4'));
});
for (const id of ['policy', 'policy-check'])
  test('round V7 policy runs the resolved node binary ' + id, async () => {
    const block = job(await read('gpu-vast.yml'), id);
    assert.match(block, /node_bin=\$\(command -v node\)/);
    assert.match(block, /"\$node_bin" "\$temporary\/trigger-policy\.mjs"/);
  });
test('round V7 independent policy check runs no third-party code', async () => {
  const yaml = await read('gpu-vast.yml');
  const block = job(yaml, 'policy-check');
  assert.doesNotMatch(block, /uses:/);
  assert.match(block, /permissions:\n      contents: read\n      pull-requests: read\n/);
  const report = job(yaml, 'report');
  assert.match(report, /needs: \[pending, policy, policy-check, gpu\]/);
  assert.match(report, /POLICY_CHECK_RUN: \$\{\{ needs\.policy-check\.outputs\.run \}\}/);
});
async function report(env) {
  const yaml = await read('gpu-vast.yml');
  const script = job(yaml, 'report')
    .split('        run: |\n')[1]
    .split('\n')
    .map((x) => (x.startsWith('          ') ? x.slice(10) : x))
    .join('\n');
  const directory = await mkdtemp(join(tmpdir(), 'v7-report-'));
  try {
    await writeFile(
      join(directory, 'curl'),
      '#!/usr/bin/env python3\nimport json,os,sys\na=sys.argv[1:]\n' +
        "b=json.load(open(a[a.index('--data-binary')+1][1:]))\n" +
        "open(os.environ['STATUS_LOG'],'a').write(json.dumps(b)+'\\n')\n",
      { mode: 0o700 },
    );
    const run = spawnSync('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script], {
      encoding: 'utf8',
      timeout: 5000,
      env: {
        PATH: directory + ':' + process.env.PATH,
        GH_TOKEN: 'mock-status-token',
        TARGET_SHA: 'a'.repeat(40),
        GITHUB_REPOSITORY: 'reindertpelsma/kayfabe',
        GITHUB_SERVER_URL: 'https://github.com',
        GITHUB_RUN_ID: '123',
        STATUS_LOG: join(directory, 'statuses'),
        PENDING_RESULT: 'success',
        POLICY_RESULT: 'success',
        POLICY_RUN: '0',
        GPU_RESULT: 'skipped',
        BROKER_STATE: '',
        BROKER_VERDICT: '',
        BROKER_VERIFIED: '',
        ...env,
      },
    });
    const statuses = (await readFile(join(directory, 'statuses'), 'utf8')).trim().split('\n');
    return { status: run.status, state: JSON.parse(statuses.at(-1)).state };
  } finally {
    await rm(directory, { recursive: true });
  }
}
for (const [label, check] of [
  ['disagrees', { POLICY_CHECK_RESULT: 'success', POLICY_CHECK_RUN: '1' }],
  ['failed', { POLICY_CHECK_RESULT: 'failure', POLICY_CHECK_RUN: '0' }],
  ['skipped', { POLICY_CHECK_RESULT: 'skipped', POLICY_CHECK_RUN: '' }],
  ['missing', {}],
])
  test('round V7 policy job alone cannot post NOT_NEEDED success ' + label, async () => {
    const r = await report(check);
    assert.notEqual(r.state, 'success');
    assert.notEqual(r.status, 0);
  });
test('round V7 both independent policy decisions post NOT_NEEDED success', async () => {
  const r = await report({ POLICY_CHECK_RESULT: 'success', POLICY_CHECK_RUN: '0' });
  assert.equal(r.state, 'success');
  assert.equal(r.status, 0);
});
// Owner 19:10/19:25: only pull_request_target and owner pushes to master rent; no SHA input.
test('round V7 GPU workflow has no dispatch trigger or SHA input', async () => {
  const yaml = await read('gpu-vast.yml');
  assert.doesNotMatch(yaml, /^  workflow_dispatch:/m);
  assert.doesNotMatch(yaml, /inputs/);
});
test('round V7 sweep workflow has no tested SHA input', async () => {
  assert.doesNotMatch(await read('gpu-vast-sweep.yml'), /tested_sha/);
});
test('round V7 sweep inputs are exactly the die preferences', async () => {
  const inputs = (await read('gpu-vast-sweep.yml'))
    .split('    inputs:\n')[1]
    .split('\npermissions:')[0];
  assert.deepEqual(
    [...inputs.matchAll(/^      ([\w-]+):$/gm)].map((m) => m[1]),
    ['preferences'],
  );
});
