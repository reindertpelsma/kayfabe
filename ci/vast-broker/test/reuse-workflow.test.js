import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { checkWorkflows } from '../workflow-guard.js';
const text = readFileSync(
  new URL('../../../.github/workflows/gpu-vast.yml', import.meta.url),
  'utf8',
);
const section = (id) => text.split('  ' + id + ':\n')[1]?.split(/^  [a-z][a-z-]*:/m)[0] ?? '';
for (const label of ['parent proof', 'readable result'])
  test('reuse workflow executes the original named job: ' + label, () => {
    assert.match(section('pending'), /reused:.*steps.reuse.outputs.reused/);
    assert.match(section('gpu'), /needs.pending.outputs.reused == 'true'/);
    assert.match(section('gpu'), /name: Replay verified parent result/);
    assert.match(section('gpu'), /gate_reuse.py.*replay/);
    assert.match(section('gpu'), /if:.*needs.pending.outputs.reused != 'true'/);
  });
for (const label of ['final status', 'original run link'])
  test('reuse workflow preserves the status job: ' + label, () => {
    assert.match(section('report'), /REUSE_PROOF:/);
    assert.match(section('report'), /reused: same tree verified on/);
    assert.match(section('report'), /proof\[[\"']url[\"']\]/);
    assert.match(section('report'), /GPU_RESULT.*success/);
  });

for (const name of ['named replay job', 'scoped metadata permissions'])
  test('reuse guard permits ' + name, () => {
    assert.deepEqual(checkWorkflows({ 'gpu-vast.yml': text }), []);
  });
for (const permission of ['checks', 'contents'])
  test('reuse guard refuses write permission ' + permission, () => {
    assert.ok(
      checkWorkflows({
        'gpu-vast.yml': text.replace(
          '      ' + permission + ': read',
          '      ' + permission + ': write',
        ),
      }).length,
    );
  });
for (const command of ['prepare', 'steal'])
  test('reuse guard refuses changed replay command ' + command, () => {
    assert.ok(
      checkWorkflows({
        'gpu-vast.yml': text.replace('gate_reuse.py" replay', 'gate_reuse.py" ' + command),
      }).length,
    );
  });
for (const condition of ["if: needs.pending.outputs.reused == 'true'", 'if: always()'])
  test('reuse guard refuses broker condition ' + condition, () => {
    assert.ok(
      checkWorkflows({
        'gpu-vast.yml': text.replace("if: needs.pending.outputs.reused != 'true'", condition),
      }).length,
    );
  });

// Execute the actual report script. No subprocess can contact GitHub or the Worker.
for (const parent of ['b'.repeat(40), 'c'.repeat(40)])
  for (const failure of [
    '',
    'GPU_RESULT',
    'PENDING_RESULT',
    'POLICY_RESULT',
    'POLICY_CHECK_RESULT',
  ])
    test('reuse actual final report ' + parent[0] + ' ' + (failure || 'success'), async () => {
      const { mkdtemp, writeFile, readFile, rm } = await import('node:fs/promises');
      const { tmpdir } = await import('node:os');
      const { join } = await import('node:path');
      const { spawnSync } = await import('node:child_process');
      const directory = await mkdtemp(join(tmpdir(), 'wk2-report-'));
      try {
        await writeFile(
          join(directory, 'curl'),
          `#!/usr/bin/env python3
import json,os,sys
args=sys.argv[1:]
assert args[-1]=='https://api.github.com/repos/reindertpelsma/kayfabe/statuses/'+'a'*40
body=json.load(open(args[args.index('--data-binary')+1][1:]))
open(os.environ['STATUS_LOG'],'w').write(json.dumps(body))
`,
          { mode: 0o700 },
        );
        const script = section('report')
          .split('        run: |\n')[1]
          .split('\n')
          .map((line) => (line.startsWith('          ') ? line.slice(10) : line))
          .join('\n');
        const env = {
          ...process.env,
          PATH: directory + ':' + process.env.PATH,
          GH_TOKEN: 'mock-status-token',
          TARGET_SHA: 'a'.repeat(40),
          GITHUB_SERVER_URL: 'https://github.com',
          GITHUB_REPOSITORY: 'reindertpelsma/kayfabe',
          GITHUB_RUN_ID: '456',
          STATUS_LOG: join(directory, 'status.json'),
          PENDING_RESULT: 'success',
          POLICY_RESULT: 'success',
          POLICY_CHECK_RESULT: 'success',
          POLICY_RUN: '1',
          POLICY_CHECK_RUN: '1',
          GPU_RESULT: 'success',
          BROKER_STATE: '',
          BROKER_VERDICT: '',
          BROKER_VERIFIED: '',
          REUSED: 'true',
          REUSE_PROOF: JSON.stringify({
            parent,
            url: 'https://github.com/reindertpelsma/kayfabe/actions/runs/123',
          }),
        };
        if (failure) env[failure] = 'failure';
        const run = spawnSync('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', script], {
          env,
          encoding: 'utf8',
          timeout: 5000,
        });
        const status = JSON.parse(await readFile(join(directory, 'status.json'), 'utf8'));
        assert.equal(run.status, failure ? 1 : 0, run.stderr);
        assert.equal(status.state, failure ? 'error' : 'success');
        if (!failure) {
          assert.equal(status.description, 'reused: same tree verified on ' + parent.slice(0, 7));
          assert.equal(
            status.target_url,
            'https://github.com/reindertpelsma/kayfabe/actions/runs/123',
          );
        }
      } finally {
        await rm(directory, { recursive: true });
      }
    });

for (const name of ['same SHA keeps prior status', 'FF proof precedes pending overwrite'])
  test('reuse probes before posting pending: ' + name, () => {
    assert.ok(
      section('pending').indexOf('name: Find complete') <
        section('pending').indexOf('name: Post pending'),
    );
  });
