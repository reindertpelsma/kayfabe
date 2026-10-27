import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
// Final review W2 (2026-10-23): the Worker no longer decides skips. FALSIFIER: the runner or the
// report job turns a Worker-sent NOT_NEEDED (or MANUAL / NEEDS_APPROVAL) with PASS green without
// a GPU result. Only the successful trusted workflow path decision may post NOT_NEEDED success.
const job = (yaml, id) => yaml.split('  ' + id + ':\n')[1].split(/\n  [\w-]+:\n/)[0];
async function report(env) {
  const yaml = await readFile(
    new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
    'utf8',
  );
  const script = job(yaml, 'report')
    .split('        run: |\n')[1]
    .split('\n')
    .map((x) => (x.startsWith('          ') ? x.slice(10) : x))
    .join('\n');
  const directory = await mkdtemp(join(tmpdir(), 'w2-report-'));
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
        POLICY_RUN: '1',
        GPU_RESULT: 'success',
        BROKER_VERDICT: 'PASS',
        BROKER_VERIFIED: '1',
        ...env,
      },
    });
    const statuses = (await readFile(join(directory, 'statuses'), 'utf8')).trim().split('\n');
    return { status: run.status, ...JSON.parse(statuses.at(-1)) };
  } finally {
    await rm(directory, { recursive: true });
  }
}
for (const state of ['NOT_NEEDED', 'MANUAL', 'NEEDS_APPROVAL'])
  test('W2: a Worker-sent ' + state + ' + PASS never posts success', async () => {
    const r = await report({ BROKER_STATE: state });
    assert.notEqual(r.state, 'success');
    assert.notEqual(r.status, 0);
    assert.doesNotMatch(r.description, new RegExp(state));
  });
for (const state of ['GRACE', 'NONE'])
  test('W2: a verified GPU result in ' + state + ' still posts success', async () => {
    const r = await report({ BROKER_STATE: state });
    assert.equal(r.state, 'success');
    assert.equal(r.status, 0);
  });
test('W2: the workflow path policy alone may still post NOT_NEEDED success', async () => {
  const r = await report({
    POLICY_RUN: '0',
    GPU_RESULT: 'skipped',
    BROKER_STATE: '',
    BROKER_VERDICT: '',
    BROKER_VERIFIED: '',
  });
  assert.equal(r.state, 'success');
  assert.match(r.description, /NOT_NEEDED/);
});
test('W2: the runner refuses a Worker-sent NOT_NEEDED + PASS', () => {
  const p = spawnSync(
    'python3',
    [
      '-I',
      new URL('edits_runner_fixture.py', import.meta.url).pathname,
      'worker-not-needed-refused',
    ],
    { encoding: 'utf8' },
  );
  assert.equal(p.status, 0, p.stderr + p.stdout);
});
