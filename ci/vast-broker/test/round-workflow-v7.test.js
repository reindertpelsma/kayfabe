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
import { checkWorkflows } from '../workflow-guard.js';
const job = (yaml, id) => yaml.split('  ' + id + ':\n')[1].split(/\n  [\w-]+:\n/)[0];
for (const name of ['gpu-vast.yml', 'gpu-vast-sweep.yml'])
  test('round V7 every action is pinned by full commit SHA ' + name, async () => {
    const uses = [...(await read(name)).matchAll(/^\s*-?\s*uses:\s*(\S+)(.*)$/gm)];
    for (const [, ref] of uses) assert.match(ref, /^[\w.-]+\/[\w./-]+@[0-9a-f]{40}$/, ref);
  });
// Owner 2026-10-27: one action-free trusted decision; missing/failed decisions never skip.
test('one policy uses preinstalled Node with its documented import-attributes floor', async () => {
  const yaml = await read('gpu-vast.yml');
  const block = job(yaml, 'policy');
  assert.doesNotMatch(block, /uses:/);
  assert.match(block, /Node 20\.10\.0\+.*18\.20\.0/);
  assert.match(block, /node_bin=\$\(command -v node\)/);
  assert.match(block, /"\$node_bin" "\$temporary\/trigger-policy\.mjs"/);
});
test('one policy is the only trigger dependency', async () => {
  const yaml = await read('gpu-vast.yml');
  assert.doesNotMatch(yaml, /policy-check|POLICY_CHECK/);
  assert.match(job(yaml, 'report'), /needs: \[pending, policy, gpu\]/);
  assert.match(
    job(yaml, 'policy'),
    /permissions:\n      contents: read\n      pull-requests: read/,
  );
});
for (const action of ['actions/setup-node', 'actions/github-script'])
  test('one policy guard refuses even pinned action ' + action, async () => {
    const yaml = await read('gpu-vast.yml');
    const block = job(yaml, 'policy');
    const changed = block.replace(
      '    steps:',
      '    steps:\n      - uses: ' + action + '@' + 'a'.repeat(40),
    );
    assert.notDeepEqual(checkWorkflows({ 'gpu-vast.yml': yaml.replace(block, changed) }), []);
  });
for (const id of ['policy-check', 'extra-trigger'])
  test('one policy guard refuses a duplicate decision ' + id, async () => {
    const yaml = await read('gpu-vast.yml');
    const changed = yaml.replace(
      '  gpu:\n',
      '  ' + id + ':\n' + job(yaml, 'policy') + '\n  gpu:\n',
    );
    assert.notDeepEqual(checkWorkflows({ 'gpu-vast.yml': changed }), []);
  });
for (const grant of ['contents', 'pull-requests'])
  test('one policy guard keeps read-only permission ' + grant, async () => {
    const yaml = await read('gpu-vast.yml');
    const block = job(yaml, 'policy');
    const changed = block.replace(grant + ': read', grant + ': write');
    assert.notDeepEqual(checkWorkflows({ 'gpu-vast.yml': yaml.replace(block, changed) }), []);
  });
for (const id of ['gpu', 'report'])
  for (const change of ['remove', 'rename'])
    test('one policy guard requires its dependency ' + id + ' ' + change, async () => {
      const yaml = await read('gpu-vast.yml');
      const block = job(yaml, id);
      const changed = block.replace(/(needs: \[pending, )policy([,\]])/, (_, before, after) =>
        change === 'remove' ? before.slice(0, -2) + after : before + 'other' + after,
      );
      assert.notEqual(changed, block);
      assert.notDeepEqual(checkWorkflows({ 'gpu-vast.yml': yaml.replace(block, changed) }), []);
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
    return { status: run.status, ...JSON.parse(statuses.at(-1)) };
  } finally {
    await rm(directory, { recursive: true });
  }
}
for (const variant of [0, 1]) {
  test('one policy alone posts NOT_NEEDED ' + variant, async () => {
    const r = await report(
      variant ? { POLICY_CHECK_RESULT: 'failure', POLICY_CHECK_RUN: '1' } : {},
    );
    assert.equal(r.state, 'success');
    assert.equal(r.status, 0);
    assert.match(r.description, /NOT_NEEDED/);
  });
  for (const refusal of ['run', 'missing', 'failed', 'gpu-failed', 'pending-failed'])
    test('one policy refuses wrong skip ' + refusal + variant, async () => {
      const env = {
        BROKER_STATE: variant ? 'NONE' : 'UNAVAILABLE',
        BROKER_VERDICT: variant ? 'PASS' : '',
        BROKER_VERIFIED: String(variant),
      };
      if (refusal === 'run') env.POLICY_RUN = '1';
      if (refusal === 'missing') env.POLICY_RUN = '';
      if (refusal === 'failed') env.POLICY_RESULT = 'failure';
      if (refusal === 'gpu-failed') env.GPU_RESULT = variant ? 'cancelled' : 'failure';
      if (refusal === 'pending-failed') env.PENDING_RESULT = variant ? 'cancelled' : 'failure';
      const r = await report(env);
      assert.notEqual(r.state, 'success');
      assert.notEqual(r.status, 0);
    });
}
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
